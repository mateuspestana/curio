//! Unified text diffs for change history.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use similar::{ChangeTag, TextDiff};

const MAX_DIFF_CHARS: usize = 50_000;

/// Produce a line-based diff showing only the items that changed.
///
/// Each line of the normalized text is one item, so unchanged items are
/// omitted instead of being dragged along as context.
pub fn unified_diff(old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => continue,
        };
        out.push(sign);
        out.push(' ');
        out.push_str(change.to_string().trim_end_matches('\n'));
        out.push('\n');

        if out.len() >= MAX_DIFF_CHARS {
            out.push_str("\n… (diff truncado)\n");
            break;
        }
    }

    if out.is_empty() {
        "(sem diferenças textuais)".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_only_changed_items() {
        let d = unified_diff("a / x\nb / y\nc / z\n", "a / x\nb / w\nc / z\n");
        assert_eq!(d, "- b / y\n+ b / w\n");
    }
}
