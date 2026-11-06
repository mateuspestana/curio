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
use unicode_width::UnicodeWidthStr;

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
    let title = Paragraph::new(vec![Line::from(vec![
        Span::styled("  Curió  ", theme::title()),
        Span::styled(subtitle, theme::muted()),
    ])])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .style(theme::normal()),
    );
    frame.render_widget(title, area);
}

/// Junta as dicas que cabem em `width` colunas, na ordem de prioridade, e sempre
/// termina com `tail` (ex.: "? ajuda · q sair"), para nunca esconder a saída.
pub fn fit_hints(items: &[&str], tail: &str, width: usize) -> String {
    const SEP: &str = " · ";
    let mut out = String::new();
    let reserved = tail.width() + SEP.width();
    for item in items {
        let extra = if out.is_empty() { 0 } else { SEP.width() } + item.width();
        if out.width() + extra + reserved > width {
            break;
        }
        if !out.is_empty() {
            out.push_str(SEP);
        }
        out.push_str(item);
    }
    if out.is_empty() {
        tail.to_string()
    } else {
        format!("{out}{SEP}{tail}")
    }
}

/// Rodapé: dicas de teclas, ou uma mensagem passageira (`toast`) no lugar delas.
pub fn draw_footer(frame: &mut Frame, area: Rect, hints: &str, toast: Option<&str>) {
    let first = match toast {
        Some(msg) => Span::styled(msg.to_string(), theme::status_changed()),
        None => Span::styled(hints.to_string(), theme::normal()),
    };
    let text = Paragraph::new(vec![
        Line::from(first),
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

#[cfg(test)]
mod tests {
    use super::*;

    const ITEMS: [&str; 4] = ["a add", "e edit", "d del", "p pausa"];

    #[test]
    fn keeps_as_many_hints_as_fit_and_always_the_tail() {
        assert_eq!(
            fit_hints(&ITEMS, "? ajuda · q sair", 200),
            "a add · e edit · d del · p pausa · ? ajuda · q sair"
        );
        let fits = fit_hints(&ITEMS, "? ajuda · q sair", 33);
        assert_eq!(fits, "a add · e edit · ? ajuda · q sair");
        assert!(fits.width() <= 33);
        // Um a menos de largura e o "e edit" cai fora; a saída nunca some.
        assert_eq!(
            fit_hints(&ITEMS, "? ajuda · q sair", 32),
            "a add · ? ajuda · q sair"
        );
        assert_eq!(fit_hints(&ITEMS, "q sair", 3), "q sair");
    }
}
