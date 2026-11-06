//! Especificação de um monitor (campos editáveis) e sua validação.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::condition::Condition;
use crate::extract::{apply_ignore_filters, normalize_and_hash, ExtractError};
use crate::fetch::validate_headers;
use crate::jsonq;
use crate::model::{Mode, Watch, MIN_INTERVAL_SECS};

/// Campo do monitor ao qual um erro de validação se refere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecField {
    Url,
    Interval,
    Selector,
    Ignore,
    Condition,
    Headers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError {
    pub field: SpecField,
    pub message: String,
}

impl SpecError {
    fn new(field: SpecField, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WatchSpec {
    pub url: String,
    pub label: String,
    pub interval_secs: u64,
    pub mode: Mode,
    pub selector: String,
    pub ignore_regex: String,
    pub condition: String,
    pub headers: String,
    pub tags: String,
}

impl WatchSpec {
    pub fn from_watch(w: &Watch) -> Self {
        Self {
            url: w.url.clone(),
            label: w.label.clone(),
            interval_secs: w.interval_secs,
            mode: w.mode,
            selector: w.selector.clone(),
            ignore_regex: w.ignore_regex.clone(),
            condition: w.condition.clone(),
            headers: w.headers.clone(),
            tags: w.tags.clone(),
        }
    }

    /// Valida tudo que dá para validar sem rede. Não resolve `${VAR}` dos cabeçalhos.
    pub fn validate(&self) -> Result<(), SpecError> {
        use SpecField as F;
        if !(self.url.starts_with("http://") || self.url.starts_with("https://")) {
            return Err(SpecError::new(
                F::Url,
                "URL deve começar com http:// ou https://",
            ));
        }
        if self.interval_secs < MIN_INTERVAL_SECS {
            return Err(SpecError::new(
                F::Interval,
                format!("intervalo mínimo é {MIN_INTERVAL_SECS} segundos"),
            ));
        }
        self.validate_selector()?;
        apply_ignore_filters("", &self.ignore_regex)
            .map_err(|e| SpecError::new(F::Ignore, e.to_string()))?;
        Condition::parse(&self.condition)
            .map_err(|e| SpecError::new(F::Condition, e.to_string()))?;
        validate_headers(&self.headers).map_err(|e| SpecError::new(F::Headers, e))?;
        if self.mode == Mode::Js && !self.headers.trim().is_empty() {
            return Err(SpecError::new(
                F::Headers,
                "cabeçalhos não são suportados no modo js (o navegador não aceita headers extras)",
            ));
        }
        Ok(())
    }

    fn validate_selector(&self) -> Result<(), SpecError> {
        let sel = self.selector.trim();
        if sel.is_empty() {
            return Ok(());
        }
        let err = |m: String| SpecError::new(SpecField::Selector, m);
        match self.mode {
            Mode::Json => jsonq::parse_path(sel).map(|_| ()).map_err(err),
            Mode::Html | Mode::Js => {
                match normalize_and_hash(
                    "<html><body><div id=\"x\">ok</div></body></html>",
                    sel,
                    "",
                ) {
                    // Não achar nada na amostra é esperado: só importa a sintaxe.
                    Err(e @ (ExtractError::InvalidCss(_) | ExtractError::XPathUnsupported)) => {
                        Err(err(e.to_string()))
                    }
                    _ => Ok(()),
                }
            }
        }
    }

    /// Versão normalizada para gravar (tags limpas, campos aparados).
    pub fn normalized(mut self) -> Self {
        self.url = self.url.trim().to_string();
        self.label = self.label.trim().to_string();
        self.selector = self.selector.trim().to_string();
        self.tags = normalize_tags(&self.tags);
        self
    }
}

/// `"Ofertas, #Preço, ofertas"` → `"ofertas,preço"` (minúsculas, sem duplicatas).
pub fn normalize_tags(raw: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for t in raw.split([',', ' ', ';']) {
        let t = t.trim().trim_start_matches('#').to_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> WatchSpec {
        WatchSpec {
            url: "https://example.com".into(),
            interval_secs: 60,
            ..Default::default()
        }
    }

    #[test]
    fn valid_default_spec() {
        assert!(spec().validate().is_ok());
    }

    #[test]
    fn rejects_bad_url_and_interval() {
        let mut s = spec();
        s.url = "ftp://x".into();
        assert!(s.validate().is_err());
        let mut s = spec();
        s.interval_secs = 1;
        assert!(s.validate().is_err());
    }

    #[test]
    fn validates_selector_by_mode() {
        let mut s = spec();
        s.selector = "div[".into();
        assert!(s.validate().is_err());
        s.mode = Mode::Json;
        s.selector = "a[*].b".into();
        assert!(s.validate().is_ok());
        s.selector = "a[x]".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn validates_condition_headers_and_regex() {
        let mut s = spec();
        s.condition = "bobagem".into();
        assert!(s.validate().is_err());
        let mut s = spec();
        s.headers = "sem-dois-pontos".into();
        assert!(s.validate().is_err());
        let mut s = spec();
        s.ignore_regex = "(".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn js_mode_rejects_headers() {
        let mut s = spec();
        s.mode = Mode::Js;
        s.headers = "X-Token: abc".into();
        assert!(s.validate().unwrap_err().message.contains("js"));
    }

    #[test]
    fn normalizes_tags() {
        assert_eq!(
            normalize_tags(" Ofertas, #Preço ofertas;; "),
            "ofertas,preço"
        );
        assert_eq!(normalize_tags(""), "");
    }
}
