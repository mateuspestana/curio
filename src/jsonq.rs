//! Mini-consulta para respostas JSON: caminho estilo `a.b[0].c`, `items[*].price`.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::extract::ExtractError;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
    /// `*` / `[*]`: todos os elementos de um array ou valores de um objeto.
    Wild,
}

/// Lê uma chave sem colchetes (até `.` ou `[`); `]` solto indica erro de digitação.
fn bare_key(chars: &[char], i: &mut usize) -> Result<String, String> {
    let start = *i;
    while *i < chars.len() && chars[*i] != '.' && chars[*i] != '[' {
        if chars[*i] == ']' {
            return Err("caminho JSON com ']' sem '['".into());
        }
        *i += 1;
    }
    Ok(chars[start..*i].iter().collect())
}

/// Interpreta um caminho. Aceita `$` inicial opcional, `.chave`, `[n]`, `[*]`,
/// `.*` e `['chave com espaço']`.
pub fn parse_path(input: &str) -> Result<Vec<Seg>, String> {
    let s = input.trim();
    let s = s.strip_prefix('$').unwrap_or(s);
    let chars: Vec<char> = s.chars().collect();
    let mut segs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '.' => {
                i += 1;
                let key = bare_key(&chars, &mut i)?;
                if key.is_empty() {
                    return Err("caminho JSON com '.' sem chave".into());
                }
                segs.push(if key == "*" { Seg::Wild } else { Seg::Key(key) });
            }
            '[' => {
                let close = chars[i..]
                    .iter()
                    .position(|c| *c == ']')
                    .map(|p| p + i)
                    .ok_or("caminho JSON com '[' sem ']'")?;
                let inner: String = chars[i + 1..close].iter().collect();
                let inner = inner.trim();
                if inner == "*" {
                    segs.push(Seg::Wild);
                } else if let Ok(n) = inner.parse::<usize>() {
                    segs.push(Seg::Index(n));
                } else if inner.len() >= 2
                    && ((inner.starts_with('\'') && inner.ends_with('\''))
                        || (inner.starts_with('"') && inner.ends_with('"')))
                {
                    segs.push(Seg::Key(inner[1..inner.len() - 1].to_string()));
                } else {
                    return Err(format!("índice JSON inválido: [{inner}]"));
                }
                i = close + 1;
            }
            _ if segs.is_empty() => {
                // Primeira chave sem ponto: `data.items`.
                let key = bare_key(&chars, &mut i)?;
                segs.push(if key == "*" { Seg::Wild } else { Seg::Key(key) });
            }
            c => return Err(format!("caminho JSON inesperado perto de '{c}'")),
        }
    }
    Ok(segs)
}

fn join_key(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// Resolve o caminho e devolve cada valor encontrado com seu caminho concreto.
pub fn select<'a>(root: &'a Value, segs: &[Seg]) -> Vec<(String, &'a Value)> {
    let mut frontier: Vec<(String, &Value)> = vec![(String::new(), root)];
    for seg in segs {
        let mut next = Vec::new();
        for (path, v) in frontier {
            match (seg, v) {
                (Seg::Key(k), Value::Object(map)) => {
                    if let Some(child) = map.get(k) {
                        next.push((join_key(&path, k), child));
                    }
                }
                (Seg::Index(n), Value::Array(arr)) => {
                    if let Some(child) = arr.get(*n) {
                        next.push((format!("{path}[{n}]"), child));
                    }
                }
                (Seg::Wild, Value::Array(arr)) => {
                    for (n, child) in arr.iter().enumerate() {
                        next.push((format!("{path}[{n}]"), child));
                    }
                }
                (Seg::Wild, Value::Object(map)) => {
                    for (k, child) in map {
                        next.push((join_key(&path, k), child));
                    }
                }
                _ => {}
            }
        }
        frontier = next;
    }
    frontier
}

fn scalar_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.split_whitespace().collect::<Vec<_>>().join(" "),
        other => other.to_string(),
    }
}

/// Achata um valor em linhas `caminho: valor` (uma por folha).
pub fn flatten(path: &str, v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) if !map.is_empty() => {
            for (k, child) in map {
                flatten(&join_key(path, k), child, out);
            }
        }
        Value::Array(arr) if !arr.is_empty() => {
            for (n, child) in arr.iter().enumerate() {
                flatten(&format!("{path}[{n}]"), child, out);
            }
        }
        _ => {
            let text = match v {
                Value::Object(_) => "{}".to_string(),
                Value::Array(_) => "[]".to_string(),
                other => scalar_text(other),
            };
            if path.is_empty() {
                out.push(text);
            } else {
                out.push(format!("{path}: {text}"));
            }
        }
    }
}

/// Corpo JSON + caminho opcional → linhas `caminho: valor`.
pub fn extract_lines(body: &str, selector: &str) -> Result<String, ExtractError> {
    let root: Value =
        serde_json::from_str(body).map_err(|e| ExtractError::InvalidJson(e.to_string()))?;
    let segs = parse_path(selector).map_err(ExtractError::InvalidJsonPath)?;
    let matches = select(&root, &segs);
    if matches.is_empty() {
        return Err(ExtractError::NoMatch);
    }
    let mut lines = Vec::new();
    for (path, v) in matches {
        flatten(&path, v, &mut lines);
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(body: &str, sel: &str) -> Vec<String> {
        extract_lines(body, sel)
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    }

    #[test]
    fn flattens_whole_document_sorted_by_key() {
        let l = lines(r#"{"b": 2, "a": {"x": "oi  mundo", "y": [1, null]}}"#, "");
        assert_eq!(
            l,
            vec!["a.x: oi mundo", "a.y[0]: 1", "a.y[1]: null", "b: 2"]
        );
    }

    #[test]
    fn selects_with_wildcard_and_index() {
        let body = r#"{"items": [{"name": "a", "price": 10}, {"name": "b", "price": 20}]}"#;
        assert_eq!(
            lines(body, "items[*].price"),
            vec!["items[0].price: 10", "items[1].price: 20"]
        );
        assert_eq!(lines(body, "$.items[1].name"), vec!["items[1].name: b"]);
        assert_eq!(
            lines(body, "items.*.name"),
            vec!["items[0].name: a", "items[1].name: b"]
        );
    }

    #[test]
    fn selects_quoted_keys_and_object_values() {
        let body = r#"{"a b": {"x": 1, "y": 2}}"#;
        assert_eq!(lines(body, "['a b'].x"), vec!["a b.x: 1"]);
        assert_eq!(lines(body, "['a b'].*"), vec!["a b.x: 1", "a b.y: 2"]);
    }

    #[test]
    fn scalar_root_and_empty_containers() {
        assert_eq!(lines("42", ""), vec!["42"]);
        assert_eq!(lines(r#"{"a": [], "b": {}}"#, ""), vec!["a: []", "b: {}"]);
    }

    #[test]
    fn errors_are_typed() {
        assert!(matches!(
            extract_lines("nao e json", ""),
            Err(ExtractError::InvalidJson(_))
        ));
        assert!(matches!(
            extract_lines("{}", "a[x]"),
            Err(ExtractError::InvalidJsonPath(_))
        ));
        assert_eq!(
            extract_lines(r#"{"a": 1}"#, "b").unwrap_err(),
            ExtractError::NoMatch
        );
    }

    #[test]
    fn parse_path_rejects_garbage() {
        assert!(parse_path("a..b").is_err());
        assert!(parse_path("a[0").is_err());
        assert!(parse_path("a]b").is_err());
        assert_eq!(parse_path("").unwrap(), vec![]);
        assert_eq!(
            parse_path("a[0].b").unwrap(),
            vec![Seg::Key("a".into()), Seg::Index(0), Seg::Key("b".into())]
        );
    }
}
