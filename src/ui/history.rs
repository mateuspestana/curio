//! Change history / diff viewer.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::model::Change;
use crate::ui::theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    changes: &[Change],
    selected: usize,
    scroll: u16,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let items: Vec<ListItem> = changes
        .iter()
        .map(|c| {
            let mark = if c.seen { " " } else { "●" };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{mark} "), theme::status_changed()),
                Span::styled(
                    c.detected_at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string(),
                    theme::normal(),
                ),
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

    let lines: Vec<Line> = diff
        .lines()
        .map(|line| {
            let style = if line.starts_with('+') {
                theme::status_ok()
            } else if line.starts_with('-') {
                theme::status_err()
            } else {
                theme::muted()
            };
            Line::from(Span::styled(line.to_string(), style))
        })
        .collect();

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0))
        .block(
            Block::default()
                .title(" Diferença ")
                .title_style(theme::panel_title())
                .borders(Borders::ALL)
                .border_style(theme::border())
                .style(theme::normal()),
        );
    frame.render_widget(para, chunks[1]);
}
