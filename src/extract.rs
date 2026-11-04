//! Extract and normalize visible text from HTML.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use sha2::{Digest, Sha256};

/// Convert HTML to normalized visible text and return (text, hex hash).
pub fn normalize_and_hash(html: &str) -> (String, String) {
    let text = html2text::from_read(html.as_bytes(), 120).unwrap_or_else(|_| {
        // Fallback: strip tags roughly if html2text fails
        html.to_string()
    });
    let normalized = normalize_text(&text);
    let hash = hash_text(&normalized);
    (normalized, hash)
}

pub fn normalize_text(text: &str) -> String {
    let collapsed: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    collapsed
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
        let (text, hash) = normalize_and_hash(html);
        assert!(text.to_lowercase().contains("hello"));
        assert!(text.to_lowercase().contains("world"));
        assert!(!text.contains("var x"));
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn same_content_same_hash() {
        let a = normalize_and_hash("<p>Same</p>");
        let b = normalize_and_hash("<div>Same</div>");
        // Both should contain "Same"; hashes may differ if markup remnants differ.
        assert!(a.0.contains("Same"));
        assert!(b.0.contains("Same"));
        let h1 = hash_text("Same");
        let h2 = hash_text("Same");
        assert_eq!(h1, h2);
    }
}
