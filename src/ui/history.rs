//! Change history / diff viewer (unificado ou lado a lado, com ou sem contexto).
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::diff::{changed_only, count_changes, side_by_side, RowKind, SideRow};
use crate::model::Change;
use crate::ui::theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Como o diff é exibido (ambos os modos são persistidos nas configurações).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffView {
    pub context: bool,
    pub side_by_side: bool,
}

const SEPARATOR: &str = " │ ";

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    changes: &[Change],
    selected: usize,
    scroll: u16,
    view: DiffView,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let items: Vec<ListItem> = changes
        .iter()
        .map(|c| {
            let mark = if c.seen { " " } else { "●" };
            let (added, removed) = count_changes(&c.diff_text);
            ListItem::new(Line::from(vec![
                Span::styled(format!("{mark} "), theme::status_changed()),
                Span::styled(
                    c.detected_at
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    theme::normal(),
                ),
                Span::styled(format!("  +{added} −{removed}"), theme::muted()),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    if !changes.is_empty() {
        state.select(Some(selected.min(changes.len() - 1)));
    }

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" Histórico — {title} "))
                .title_style(theme::panel_title())
                .borders(Borders::ALL)
                .border_style(theme::border())
                .style(theme::normal()),
        )
        .highlight_style(theme::selected());
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let diff = changes
        .get(selected)
        .map(|c| c.diff_text.as_str())
        .unwrap_or("Selecione uma mudança.");

    let yes_no = |b: bool| if b { "sim" } else { "não" };
    let pane_title = format!(
        " Diferença · contexto [c]: {} · lado a lado [s]: {} ",
        yes_no(view.context),
        yes_no(view.side_by_side)
    );
    let block = Block::default()
        .title(pane_title)
        .title_style(theme::panel_title())
        .borders(Borders::ALL)
        .border_style(theme::border())
        .style(theme::normal());

    let para = if view.side_by_side {
        let inner_width = chunks[1].width.saturating_sub(2) as usize;
        let rows = side_by_side(diff, view.context);
        Paragraph::new(side_lines(&rows, inner_width)).scroll((scroll, 0))
    } else {
        let shown = if view.context {
            diff.to_string()
        } else {
            changed_only(diff)
        };
        let lines: Vec<Line> = shown
            .lines()
            .map(|line| Line::from(Span::styled(line.to_string(), unified_style(line))))
            .collect();
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
    };
    frame.render_widget(para.block(block), chunks[1]);
}

fn unified_style(line: &str) -> Style {
    if line.starts_with('+') {
        theme::status_ok()
    } else if line.starts_with('-') {
        theme::status_err()
    } else {
        theme::muted()
    }
}

/// Quebra `text` em linhas de no máximo `width` colunas (largura de exibição).
pub fn wrap_cell(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    for c in text.chars() {
        let cw = c.width().unwrap_or(0);
        if cur_w + cw > width && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push(c);
        cur_w += cw;
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

fn pad_to(s: &str, width: usize) -> String {
    let w = s.width();
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

/// Monta as linhas `removido │ adicionado`; cada linha lógica ocupa a mesma altura
/// nos dois lados, então a rolagem única mantém tudo alinhado.
pub fn side_lines(rows: &[SideRow], total_width: usize) -> Vec<Line<'static>> {
    let col = total_width.saturating_sub(SEPARATOR.width()) / 2;
    let col = col.max(4);
    let mut out = Vec::new();

    for row in rows {
        if row.kind == RowKind::Gap {
            out.push(Line::from(Span::styled(
                format!(
                    "{} ⋯ {}",
                    "─".repeat(col.saturating_sub(2)),
                    "─".repeat(col)
                ),
                theme::muted(),
            )));
            continue;
        }
        let (left_style, right_style) = match row.kind {
            RowKind::Change => (theme::status_err(), theme::status_ok()),
            _ => (theme::muted(), theme::muted()),
        };
        let left = wrap_cell(row.left.as_deref().unwrap_or(""), col);
        let right = wrap_cell(row.right.as_deref().unwrap_or(""), col);
        for i in 0..left.len().max(right.len()) {
            let l = left.get(i).map(String::as_str).unwrap_or("");
            let r = right.get(i).map(String::as_str).unwrap_or("");
            out.push(Line::from(vec![
                Span::styled(pad_to(l, col), left_style),
                Span::styled(SEPARATOR, theme::muted()),
                Span::styled(r.to_string(), right_style),
            ]));
        }
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled(
            "(sem alterações a mostrar — tente ligar o contexto com [c])",
            theme::muted(),
        )));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn wrap_cell_respects_display_width() {
        assert_eq!(wrap_cell("abcdef", 4), vec!["abcd", "ef"]);
        assert_eq!(wrap_cell("", 4), vec![""]);
        // Caracteres largos (CJK) ocupam 2 colunas.
        assert_eq!(wrap_cell("日本語", 4), vec!["日本", "語"]);
        assert!(wrap_cell("ação não", 3).iter().all(|l| l.width() <= 3));
    }

    #[test]
    fn side_lines_align_both_columns_when_one_side_wraps() {
        let rows = vec![SideRow {
            left: Some("0123456789".into()),
            right: Some("ab".into()),
            kind: RowKind::Change,
        }];
        // largura total 23: separador 3 → colunas de 10.
        let lines = side_lines(&rows, 23);
        assert_eq!(lines.len(), 1);
        assert_eq!(text(&lines[0]), "0123456789 │ ab");

        let rows = vec![SideRow {
            left: Some("0123456789ABCDEF".into()),
            right: Some("ab".into()),
            kind: RowKind::Change,
        }];
        let lines = side_lines(&rows, 23);
        assert_eq!(lines.len(), 2, "o lado mais longo define a altura");
        assert_eq!(text(&lines[0]), "0123456789 │ ab");
        assert_eq!(text(&lines[1]), "ABCDEF     │ ");
    }

    #[test]
    fn side_lines_explain_empty_view() {
        let lines = side_lines(&[], 40);
        assert!(text(&lines[0]).contains("sem alterações"));
    }
}
