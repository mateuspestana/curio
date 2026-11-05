//! Extract and normalize visible text from HTML (optional CSS/XPath + ignore filters).
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use regex::Regex;
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExtractError {
    #[error("seletor CSS inválido: {0}")]
    InvalidCss(String),
    #[error("XPath não suportado diretamente: use CSS equivalente (ex.: #id, .classe, div > p)")]
    XPathUnsupported,
    #[error("regex de ignorar inválida: {0}")]
    InvalidIgnoreRegex(String),
    #[error("seletor não encontrou elementos")]
    NoMatch,
}

/// Largura de renderização do texto usado no hash (histórico; não alterar, ou
/// todos os monitores existentes registrariam uma mudança falsa).
const HASH_WIDTH: usize = 120;
/// Largura "infinita" para o texto exibido: evita quebrar um item em duas linhas.
const DISPLAY_WIDTH: usize = 100_000;

/// Extract text + hash from HTML with optional selector and ignore patterns.
///
/// O hash é calculado sobre o texto com espaços colapsados (como nas versões
/// anteriores). O texto retornado preserva uma linha por item, para que o
/// diff mostre só os itens que mudaram.
pub fn normalize_and_hash(
    html: &str,
    selector: &str,
    ignore_regex: &str,
) -> Result<(String, String), ExtractError> {
    let raw = extract_raw_text(html, selector, HASH_WIDTH)?;
    let filtered = apply_ignore_filters(&raw, ignore_regex)?;
    let hash = hash_text(&normalize_text(&filtered));

    let raw_wide = extract_raw_text(html, selector, DISPLAY_WIDTH)?;
    let filtered_wide = apply_ignore_filters(&raw_wide, ignore_regex)?;
    Ok((normalize_lines(&filtered_wide), hash))
}

fn extract_raw_text(html: &str, selector: &str, width: usize) -> Result<String, ExtractError> {
    let sel = selector.trim();
    if sel.is_empty() {
        let text = html2text::from_read(html.as_bytes(), width).unwrap_or_else(|_| html.to_string());
        return Ok(text);
    }

    if looks_like_xpath(sel) {
        if let Some(css) = xpath_to_css_hint(sel) {
            return extract_with_css(html, &css);
        }
        return Err(ExtractError::XPathUnsupported);
    }

    extract_with_css(html, sel)
}

fn extract_with_css(html: &str, css: &str) -> Result<String, ExtractError> {
    let parsed = Selector::parse(css).map_err(|e| ExtractError::InvalidCss(e.to_string()))?;
    let document = Html::parse_document(html);
    let mut parts = Vec::new();
    for el in document.select(&parsed) {
        let t = el.text().collect::<Vec<_>>().join(" ");
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if !t.is_empty() {
            parts.push(t);
        }
    }
    if parts.is_empty() {
        return Err(ExtractError::NoMatch);
    }
    Ok(parts.join("\n"))
}

fn looks_like_xpath(s: &str) -> bool {
    s.starts_with('/') || s.starts_with('(') || s.contains("//")
}

/// Best-effort conversion of a few common XPath forms to CSS.
fn xpath_to_css_hint(xpath: &str) -> Option<String> {
    let x = xpath.trim();
    // //*[@id='foo'] or //*[@id="foo"]
    let id_re = Regex::new(r#"//\*\[@id=['\"]([^'\"]+)['\"]\]"#).ok()?;
    if let Some(c) = id_re.captures(x) {
        return Some(format!("#{}", &c[1]));
    }
    // //tag[@id='foo']
    let tag_id = Regex::new(r#"//([a-zA-Z][\w-]*)\[@id=['\"]([^'\"]+)['\"]\]"#).ok()?;
    if let Some(c) = tag_id.captures(x) {
        return Some(format!("{}#{}", &c[1], &c[2]));
    }
    // //tag[@class='foo']
    let tag_class = Regex::new(r#"//([a-zA-Z][\w-]*)\[@class=['\"]([^'\"]+)['\"]\]"#).ok()?;
    if let Some(c) = tag_class.captures(x) {
        let classes = c[2].split_whitespace().collect::<Vec<_>>().join(".");
        return Some(format!("{}.{}", &c[1], classes));
    }
    // //div/p → div > p (very naive single path)
    if x.starts_with("//") && !x.contains('[') && !x.contains('*') {
        let path = x.trim_start_matches('/');
        if path
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '/' || ch == '-' || ch == '_')
        {
            return Some(path.replace('/', " > "));
        }
    }
    None
}

/// Drop lines that match any non-empty ignore regex (one pattern per line).
pub fn apply_ignore_filters(text: &str, patterns: &str) -> Result<String, ExtractError> {
    let mut regs = Vec::new();
    for line in patterns.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let re = Regex::new(line).map_err(|e| ExtractError::InvalidIgnoreRegex(e.to_string()))?;
        regs.push(re);
    }
    if regs.is_empty() {
        return Ok(text.to_string());
    }
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| !regs.iter().any(|r| r.is_match(line)))
        .collect();
    Ok(kept.join("\n"))
}

pub fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Colapsa espaços dentro de cada linha e descarta linhas vazias.
pub fn normalize_lines(text: &str) -> String {
    text.lines()
        .map(normalize_text)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_whitespace() {
        assert_eq!(normalize_text("  a \n\t b  "), "a b");
    }

    #[test]
    fn extracts_visible_text_ignoring_scripts() {
        let html = r#"
            <html><head><script>var x=1;</script><style>.x{}</style></head>
            <body><h1>Hello</h1><p>World</p></body></html>
        "#;
        let (text, hash) = normalize_and_hash(html, "", "").unwrap();
        assert!(text.to_lowercase().contains("hello"));
        assert!(text.to_lowercase().contains("world"));
        assert!(!text.contains("var x"));
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn keeps_one_line_per_item_but_hash_ignores_layout() {
        let a = "<ul><li>NVIDIA / auto-ontology</li><li>max-sxty / worktrunk</li></ul>";
        let (text, hash_a) = normalize_and_hash(a, "", "").unwrap();
        assert_eq!(text.lines().count(), 2);
        let b = "<ul><li>NVIDIA / auto-ontology</li>\n\n<li>max-sxty / worktrunk</li></ul>";
        let (_, hash_b) = normalize_and_hash(b, "", "").unwrap();
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn css_selector_limits_scope() {
        let html = r#"<html><body><div id="a">Alpha</div><div id="b">Beta</div></body></html>"#;
        let (text, _) = normalize_and_hash(html, "#a", "").unwrap();
        assert!(text.contains("Alpha"));
        assert!(!text.contains("Beta"));
    }

    #[test]
    fn xpath_id_converts_to_css() {
        let html = r#"<html><body><div id="price">R$ 10</div><p>noise</p></body></html>"#;
        let (text, _) = normalize_and_hash(html, "//*[@id='price']", "").unwrap();
        assert!(text.contains("R$ 10"));
        assert!(!text.contains("noise"));
    }

    #[test]
    fn ignore_regex_drops_lines() {
        let text = "keep\ntimestamp: 123\nalso keep";
        let out = apply_ignore_filters(text, r"timestamp:").unwrap();
        assert!(out.contains("keep"));
        assert!(!out.contains("timestamp"));
    }
}
