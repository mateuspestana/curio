//! Pipeline único de obtenção de conteúdo (monitor e prévia): HTTP com
//! cabeçalhos, JSON ou página renderizada por navegador headless.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::extract::extract;
use crate::model::{Mode, Watch, BROWSER_TIMEOUT_SECS, FETCH_TIMEOUT_SECS, MAX_RETRY_AFTER_SECS};
use chrono::Utc;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, RETRY_AFTER};
use reqwest::Client;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Semaphore;

/// Navegadores headless são pesados: no máximo uma renderização por vez.
static BROWSER_LOCK: Semaphore = Semaphore::const_new(1);

/// Virtual time do Chrome: tempo simulado para o JavaScript da página rodar.
const BROWSER_VIRTUAL_TIME_MS: u32 = 10_000;

#[derive(Debug, Clone)]
pub struct FetchError {
    pub message: String,
    /// Espera pedida pelo servidor (`Retry-After`), em segundos.
    pub retry_after_secs: Option<u64>,
}

impl FetchError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retry_after_secs: None,
        }
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Tudo que define *o que* buscar e *como* extrair o texto.
#[derive(Debug, Clone)]
pub struct Target {
    pub url: String,
    pub mode: Mode,
    pub selector: String,
    pub ignore_regex: String,
    pub headers: String,
}

impl Target {
    pub fn from_watch(w: &Watch) -> Self {
        Self {
            url: w.url.clone(),
            mode: w.mode,
            selector: w.selector.clone(),
            ignore_regex: w.ignore_regex.clone(),
            headers: w.headers.clone(),
        }
    }
}

/// Texto normalizado (uma linha por item) e hash para comparação.
#[derive(Debug, Clone)]
pub struct Extracted {
    pub text: String,
    pub hash: String,
}

pub fn user_agent() -> String {
    format!(
        "Curio/{} (+MACAPE; https://github.com/mateuspestana/curio)",
        env!("CARGO_PKG_VERSION")
    )
}

pub fn build_client() -> Client {
    Client::builder()
        .user_agent(user_agent())
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
        .expect("http client")
}

/// Busca a URL e extrai o texto: único caminho usado pelo monitor e pela prévia.
pub async fn fetch_and_extract(client: &Client, target: &Target) -> Result<Extracted, FetchError> {
    let body = fetch_body(client, target).await?;
    let (text, hash) = extract(&body, target.mode, &target.selector, &target.ignore_regex)
        .map_err(|e| FetchError::new(e.to_string()))?;
    Ok(Extracted { text, hash })
}

async fn fetch_body(client: &Client, target: &Target) -> Result<String, FetchError> {
    match target.mode {
        Mode::Js => fetch_rendered(&target.url).await,
        Mode::Html | Mode::Json => fetch_http(client, target).await,
    }
}

async fn fetch_http(client: &Client, target: &Target) -> Result<String, FetchError> {
    let headers = build_header_map(&target.headers).map_err(FetchError::new)?;
    let resp = client
        .get(&target.url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| FetchError::new(format!("rede: {}", e.without_url())))?;
    let status = resp.status();
    if !status.is_success() {
        let retry_after_secs = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after);
        return Err(FetchError {
            message: format!("HTTP {status}"),
            retry_after_secs,
        });
    }
    resp.text()
        .await
        .map_err(|e| FetchError::new(format!("ler corpo: {}", e.without_url())))
}

// ---------------------------------------------------------------- Retry-After

/// `Retry-After` em segundos (`120`) ou data HTTP (`Wed, 21 Oct 2026 07:28:00 GMT`).
pub fn parse_retry_after(value: &str) -> Option<u64> {
    let value = value.trim();
    let secs = match value.parse::<u64>() {
        Ok(n) => n,
        Err(_) => {
            let when = chrono::DateTime::parse_from_rfc2822(value).ok()?;
            (when.with_timezone(&Utc) - Utc::now()).num_seconds().max(0) as u64
        }
    };
    Some(secs.min(MAX_RETRY_AFTER_SECS))
}

// ------------------------------------------------------------------ cabeçalhos

/// Troca `${VAR}` pelo valor da variável de ambiente; erro se ela não existir.
pub fn expand_env_with(
    value: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| "cabeçalho com \"${\" sem \"}\"".to_string())?;
        let name = after[..end].trim();
        let val = lookup(name)
            .ok_or_else(|| format!("variável de ambiente {name} não definida (cabeçalho)"))?;
        out.push_str(&val);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn split_header_line(line: &str) -> Result<(&str, &str), String> {
    let (name, value) = line
        .split_once(':')
        .ok_or_else(|| format!("cabeçalho inválido \"{line}\" (use Nome: valor)"))?;
    Ok((name.trim(), value.trim()))
}

/// Confere a sintaxe (nomes válidos) sem resolver variáveis de ambiente.
pub fn validate_headers(raw: &str) -> Result<(), String> {
    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (name, _) = split_header_line(line)?;
        HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("nome de cabeçalho inválido: \"{name}\""))?;
    }
    Ok(())
}

/// Monta os cabeçalhos expandindo `${VAR}` agora, na hora do pedido: o valor
/// expandido nunca é gravado no banco, em erros nem em exportações.
pub fn build_header_map(raw: &str) -> Result<HeaderMap, String> {
    build_header_map_with(raw, |n| std::env::var(n).ok())
}

pub fn build_header_map_with(
    raw: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<HeaderMap, String> {
    let mut map = HeaderMap::new();
    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (name, value) = split_header_line(line)?;
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("nome de cabeçalho inválido: \"{name}\""))?;
        let expanded = expand_env_with(value, &lookup)?;
        let mut hv = HeaderValue::from_str(&expanded)
            .map_err(|_| format!("valor inválido para o cabeçalho {name}"))?;
        hv.set_sensitive(true);
        map.append(name, hv);
    }
    Ok(map)
}

// ------------------------------------------------------------------- navegador

/// Procura um navegador Chromium: `CURIO_CHROME`, PATH, apps do macOS, Windows.
pub fn find_browser() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CURIO_CHROME").map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    const NAMES: [&str; 8] = [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
        "microsoft-edge",
        "brave-browser",
        "brave",
    ];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in NAMES {
                for candidate in [dir.join(name), dir.join(format!("{name}.exe"))] {
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    let mut fixed: Vec<PathBuf> = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    for var in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(var).map(PathBuf::from) {
            fixed.push(base.join("Google/Chrome/Application/chrome.exe"));
            fixed.push(base.join("Microsoft/Edge/Application/msedge.exe"));
        }
    }
    fixed.into_iter().find(|p| p.is_file())
}

/// Renderiza a página com `--dump-dom` e devolve o DOM final.
async fn fetch_rendered(url: &str) -> Result<String, FetchError> {
    let browser = find_browser().ok_or_else(|| {
        FetchError::new(
            "navegador não encontrado para o modo js (instale Chrome/Chromium ou defina CURIO_CHROME)",
        )
    })?;
    // Perfil próprio: o Chrome do usuário (aberto) travaria o perfil padrão.
    let profile = crate::db::data_dir()
        .map_err(|e| FetchError::new(format!("diretório de dados: {e}")))?
        .join("browser-profile");
    std::fs::create_dir_all(&profile)
        .map_err(|e| FetchError::new(format!("criar perfil do navegador: {e}")))?;

    let _permit = BROWSER_LOCK
        .acquire()
        .await
        .map_err(|_| FetchError::new("fila do navegador encerrada"))?;

    let mut cmd = Command::new(&browser);
    cmd.arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .arg("--mute-audio")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--user-agent={}", crate::fetch::user_agent()))
        .arg(format!("--virtual-time-budget={BROWSER_VIRTUAL_TIME_MS}"))
        .arg("--dump-dom")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);

    let output = tokio::time::timeout(Duration::from_secs(BROWSER_TIMEOUT_SECS), cmd.output())
        .await
        .map_err(|_| FetchError::new(format!("navegador excedeu {BROWSER_TIMEOUT_SECS}s")))?
        .map_err(|e| FetchError::new(format!("iniciar navegador: {e}")))?;
    if !output.status.success() {
        return Err(FetchError::new(format!(
            "navegador saiu com {}",
            output.status
        )));
    }
    let dom = String::from_utf8_lossy(&output.stdout).into_owned();
    if dom.trim().is_empty() {
        return Err(FetchError::new("navegador não devolveu conteúdo"));
    }
    Ok(dom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_seconds_and_cap() {
        assert_eq!(parse_retry_after("120"), Some(120));
        assert_eq!(parse_retry_after(" 7 "), Some(7));
        assert_eq!(parse_retry_after("999999999"), Some(MAX_RETRY_AFTER_SECS));
        assert_eq!(parse_retry_after("quando der"), None);
    }

    #[test]
    fn retry_after_http_date() {
        let future = (Utc::now() + chrono::Duration::seconds(90))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        let secs = parse_retry_after(&future).unwrap();
        assert!((85..=90).contains(&secs), "got {secs}");
        // Data no passado: não espera.
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), Some(0));
    }

    #[test]
    fn expands_env_placeholders() {
        let lookup = |n: &str| (n == "TOKEN").then(|| "abc".to_string());
        assert_eq!(
            expand_env_with("Bearer ${TOKEN}!", lookup).unwrap(),
            "Bearer abc!"
        );
        assert_eq!(
            expand_env_with("sem variável", lookup).unwrap(),
            "sem variável"
        );
        let err = expand_env_with("${OUTRA}", lookup).unwrap_err();
        assert!(err.contains("OUTRA"));
        assert!(expand_env_with("${aberto", lookup).is_err());
    }

    #[test]
    fn builds_headers_and_never_leaks_missing_secret() {
        let lookup = |n: &str| (n == "T").then(|| "s3".to_string());
        let map = build_header_map_with("Authorization: Bearer ${T}\n\nX-A: 1", lookup).unwrap();
        assert_eq!(map["authorization"], "Bearer s3");
        assert!(map["authorization"].is_sensitive());
        assert_eq!(map["x-a"], "1");
        assert!(build_header_map_with("Authorization: ${FALTA}", lookup).is_err());
        assert!(build_header_map_with("sem-dois-pontos", lookup).is_err());
    }

    #[test]
    fn validates_header_syntax_only() {
        assert!(validate_headers("Authorization: ${NAO_DEFINIDA}").is_ok());
        assert!(validate_headers("Nome Inválido: x").is_err());
        assert!(validate_headers("").is_ok());
    }
}
