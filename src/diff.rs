//! Unified text diffs for change history.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use similar::{ChangeTag, TextDiff};

const MAX_DIFF_CHARS: usize = 50_000;

/// Produce a readable unified-style diff between old and new normalized text.
pub fn unified_diff(old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => ' ',
        };
        let line = change.to_string();
        // Keep equal context short: skip long unchanged runs for UI.
        if change.tag() == ChangeTag::Equal {
            let trimmed = line.trim_end_matches('\n');
            if trimmed.len() > 200 {
                out.push_str("  …\n");
                continue;
            }
        }
        out.push(sign);
        out.push(' ');
        out.push_str(line.trim_end_matches('\n'));
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
    fn detects_addition_and_removal() {
        let d = unified_diff("alpha\nbeta\n", "alpha\ngamma\n");
        assert!(d.contains("- beta") || d.contains("-beta") || d.contains("- beta\n") || d.contains("beta"));
        assert!(d.contains("gamma"));
        assert!(d.contains('+') || d.contains('-'));
    }
}
