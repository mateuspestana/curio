//! Condições de alerta: só notificar quando o novo texto atende às regras.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda
//!
//! Uma regra por linha (todas precisam valer):
//!
//! ```text
//! contém: promoção          # texto presente (sem diferenciar maiúsculas)
//! não contém: esgotado      # texto ausente
//! regex: estoque:\s*[1-9]   # regex em qualquer ponto do texto
//! número < 100              # primeiro número do texto (também <=, >, >=, ==, !=)
//! ```
//!
//! O alerta é disparado na *transição*: o texto anterior não atendia e o novo
//! atende. Assim "contém: X" não repete a cada mudança irrelevante enquanto X
//! continua na página. Quem quiser alertar em toda mudança deixa o campo vazio.

use regex::Regex;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConditionError {
    #[error("regra de condição inválida: \"{0}\" (use contém:, não contém:, regex: ou número <|>|<=|>=|==|!= N)")]
    Unknown(String),
    #[error("regex de condição inválida: {0}")]
    BadRegex(String),
    #[error("número inválido na condição: \"{0}\"")]
    BadNumber(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl Cmp {
    fn eval(self, a: f64, b: f64) -> bool {
        match self {
            Self::Lt => a < b,
            Self::Le => a <= b,
            Self::Gt => a > b,
            Self::Ge => a >= b,
            Self::Eq => (a - b).abs() < 1e-9,
            Self::Ne => (a - b).abs() >= 1e-9,
        }
    }
}

#[derive(Debug, Clone)]
enum Rule {
    Contains(String),
    NotContains(String),
    Regex(Regex),
    Number(Cmp, f64),
}

impl Rule {
    fn matches(&self, text: &str, lower: &str) -> bool {
        match self {
            Self::Contains(s) => lower.contains(s.as_str()),
            Self::NotContains(s) => !lower.contains(s.as_str()),
            Self::Regex(re) => re.is_match(text),
            Self::Number(cmp, limit) => first_number(text).is_some_and(|n| cmp.eval(n, *limit)),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Condition {
    rules: Vec<Rule>,
}

/// Remove `prefix` (sem diferenciar maiúsculas) e devolve o resto, se casar.
fn strip_prefix_ci<'a>(line: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let lower = line.to_lowercase();
    for p in prefixes {
        if lower.starts_with(p) {
            // `to_lowercase` pode mudar o comprimento em bytes: recorta por chars.
            let skip = p.chars().count();
            let idx = line
                .char_indices()
                .nth(skip)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            return Some(line[idx..].trim());
        }
    }
    None
}

impl Condition {
    pub fn parse(src: &str) -> Result<Self, ConditionError> {
        let mut rules = Vec::new();
        for line in src.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            rules.push(parse_rule(line)?);
        }
        Ok(Self { rules })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn matches(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        self.rules.iter().all(|r| r.matches(text, &lower))
    }

    /// Alerta quando a condição passa de falsa para verdadeira (ou sempre, se vazia).
    pub fn should_alert(&self, prev: &str, new: &str) -> bool {
        self.is_empty() || (!self.matches(prev) && self.matches(new))
    }
}

fn parse_rule(line: &str) -> Result<Rule, ConditionError> {
    // Negativas primeiro: "não contém:" começa com o mesmo texto de "contém:".
    if let Some(rest) = strip_prefix_ci(
        line,
        &[
            "não contém:",
            "nao contem:",
            "!contém:",
            "!contem:",
            "not contains:",
        ],
    ) {
        return Ok(Rule::NotContains(rest.to_lowercase()));
    }
    if let Some(rest) = strip_prefix_ci(line, &["contém:", "contem:", "contains:"]) {
        return Ok(Rule::Contains(rest.to_lowercase()));
    }
    if let Some(rest) = strip_prefix_ci(line, &["regex:"]) {
        let re = Regex::new(rest).map_err(|e| ConditionError::BadRegex(e.to_string()))?;
        return Ok(Rule::Regex(re));
    }
    if let Some(rest) = strip_prefix_ci(line, &["número", "numero", "num"]) {
        for (op, cmp) in [
            ("<=", Cmp::Le),
            (">=", Cmp::Ge),
            ("==", Cmp::Eq),
            ("!=", Cmp::Ne),
            ("<", Cmp::Lt),
            (">", Cmp::Gt),
            ("=", Cmp::Eq),
        ] {
            if let Some(value) = rest.strip_prefix(op) {
                let value = value.trim();
                let n = parse_br_number(value)
                    .ok_or_else(|| ConditionError::BadNumber(value.to_string()))?;
                return Ok(Rule::Number(cmp, n));
            }
        }
    }
    Err(ConditionError::Unknown(line.to_string()))
}

/// Primeiro número do texto, lido no padrão brasileiro (ver [`parse_br_number`]).
pub fn first_number(text: &str) -> Option<f64> {
    let re = Regex::new(r"-?\d[\d.,]*\d|-?\d").ok()?;
    re.find(text).and_then(|m| parse_br_number(m.as_str()))
}

/// Converte `1.234,56`, `1234.56`, `R$ 10`, `1.234` etc. em `f64`.
///
/// Regras (padrão brasileiro):
/// - com ponto e vírgula, o **último** separador é o decimal (`1.234,56` e `1,234.56`);
/// - só vírgula: é sempre decimal (`1,5` = 1.5; `1,234` = 1.234);
/// - só ponto: milhar quando segue o padrão `d{1,3}(.ddd)+` e não começa em `0`
///   (`1.234` = 1234, `12.345.678` = 12345678); caso contrário é decimal
///   (`1.5` = 1.5, `0.123` = 0.123).
pub fn parse_br_number(raw: &str) -> Option<f64> {
    let trimmed = raw
        .trim()
        .trim_start_matches(|c: char| !(c.is_ascii_digit() || c == '-'));
    let (neg, rest) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed),
    };
    let body: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | ','))
        .collect();
    let body = body.trim_end_matches(['.', ',']);
    if body.is_empty() {
        return None;
    }
    let has_dot = body.contains('.');
    let has_comma = body.contains(',');
    let normalized = if has_dot && has_comma {
        let last_dot = body.rfind('.')?;
        let last_comma = body.rfind(',')?;
        if last_comma > last_dot {
            body.replace('.', "").replace(',', ".")
        } else {
            body.replace(',', "")
        }
    } else if has_comma {
        if body.matches(',').count() > 1 {
            return None;
        }
        body.replace(',', ".")
    } else if has_dot {
        let groups: Vec<&str> = body.split('.').collect();
        let thousands = groups.len() >= 2
            && !groups[0].is_empty()
            && groups[0].len() <= 3
            && !groups[0].starts_with('0')
            && groups[1..].iter().all(|g| g.len() == 3);
        if thousands {
            body.replace('.', "")
        } else if groups.len() > 2 {
            return None;
        } else {
            body.to_string()
        }
    } else {
        body.to_string()
    };
    let n: f64 = normalized.parse().ok()?;
    Some(if neg { -n } else { n })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_brazilian_numbers() {
        assert_eq!(parse_br_number("1.234,56"), Some(1234.56));
        assert_eq!(parse_br_number("R$ 1.234,56"), Some(1234.56));
        assert_eq!(parse_br_number("1,234.56"), Some(1234.56));
        assert_eq!(parse_br_number("1,5"), Some(1.5));
        assert_eq!(parse_br_number("1,234"), Some(1.234));
        assert_eq!(parse_br_number("1.234"), Some(1234.0));
        assert_eq!(parse_br_number("12.345.678"), Some(12_345_678.0));
        assert_eq!(parse_br_number("1.5"), Some(1.5));
        assert_eq!(parse_br_number("0.123"), Some(0.123));
        assert_eq!(parse_br_number("-3,5"), Some(-3.5));
        assert_eq!(parse_br_number("42"), Some(42.0));
        assert_eq!(parse_br_number("abc"), None);
    }

    #[test]
    fn first_number_skips_text() {
        assert_eq!(first_number("Preço: R$ 89,90 à vista"), Some(89.9));
        assert_eq!(first_number("sem números"), None);
        assert_eq!(first_number("até 10."), Some(10.0));
    }

    #[test]
    fn contains_rules_are_case_insensitive() {
        let c = Condition::parse("contém: Promoção\nnão contém: esgotado").unwrap();
        assert!(c.matches("Grande PROMOÇÃO hoje"));
        assert!(!c.matches("Grande promoção, esgotado"));
        assert!(!c.matches("nada"));
    }

    #[test]
    fn regex_and_number_rules() {
        let c = Condition::parse("regex: estoque:\\s*[1-9]\nnúmero < 100").unwrap();
        // A regra numérica usa o PRIMEIRO número do texto.
        assert!(c.matches("preço 80 estoque: 5"));
        assert!(!c.matches("preço 80 estoque: 0"));
        assert!(!c.matches("preço 150 estoque: 5"));
        let ge = Condition::parse("num >= 1.000,50").unwrap();
        assert!(ge.matches("1.000,50"));
        assert!(!ge.matches("999"));
    }

    #[test]
    fn missing_number_never_matches() {
        let c = Condition::parse("número > 0").unwrap();
        assert!(!c.matches("sem número"));
    }

    #[test]
    fn alerts_only_on_false_to_true_transition() {
        let c = Condition::parse("contém: disponível").unwrap();
        assert!(c.should_alert("esgotado", "agora disponível"));
        assert!(!c.should_alert("disponível (10)", "disponível (9)"));
        assert!(!c.should_alert("esgotado", "ainda esgotado"));
        let price = Condition::parse("número < 100").unwrap();
        assert!(price.should_alert("R$ 120", "R$ 90"));
        assert!(!price.should_alert("R$ 90", "R$ 85"));
    }

    #[test]
    fn empty_condition_always_alerts() {
        let c = Condition::parse("\n# só comentário\n").unwrap();
        assert!(c.is_empty());
        assert!(c.should_alert("a", "b"));
    }

    #[test]
    fn rejects_bad_rules() {
        assert!(matches!(
            Condition::parse("bobagem"),
            Err(ConditionError::Unknown(_))
        ));
        assert!(matches!(
            Condition::parse("regex: ("),
            Err(ConditionError::BadRegex(_))
        ));
        assert!(matches!(
            Condition::parse("número < abc"),
            Err(ConditionError::BadNumber(_))
        ));
    }
}
