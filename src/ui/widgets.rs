//! Shared widgets.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::model::AUTHOR_SIGNATURE;
use crate::ui::theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn main_chunks(area: Rect) -> (Rect, Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);
    (chunks[0], chunks[1], chunks[2])
}

pub fn draw_header(frame: &mut Frame, area: Rect, subtitle: &str) {
    let title = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("  Curió  ", theme::title()),
            Span::styled(subtitle, theme::muted()),
        ]),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .style(theme::normal()),
    );
    frame.render_widget(title, area);
}

pub fn draw_footer(frame: &mut Frame, area: Rect, hints: &str) {
    let text = Paragraph::new(vec![
        Line::from(Span::styled(hints, theme::normal())),
        Line::from(Span::styled(AUTHOR_SIGNATURE, theme::muted())),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .style(theme::normal()),
    );
    frame.render_widget(text, area);
}
