//! Visual theme for Curió (dark + amber accent — not purple).
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use ratatui::style::{Color, Modifier, Style};

pub const BG: Color = Color::Rgb(14, 16, 20);
pub const FG: Color = Color::Rgb(240, 242, 245);
/// Secondary text — kept bright enough for dark backgrounds.
pub const MUTED: Color = Color::Rgb(168, 176, 188);
pub const ACCENT: Color = Color::Rgb(242, 180, 64); // amber
pub const OK: Color = Color::Rgb(96, 200, 130);
#[allow(dead_code)]
pub const WARN: Color = Color::Rgb(242, 180, 64);
pub const ERR: Color = Color::Rgb(235, 110, 110);
pub const CHANGED: Color = Color::Rgb(110, 200, 215); // teal
/// Borders: visible against BG without competing with titles.
pub const BORDER: Color = Color::Rgb(110, 120, 135);

pub fn title() -> Style {
    Style::default()
        .fg(ACCENT)
        .bg(BG)
        .add_modifier(Modifier::BOLD)
}

/// Panel / field titles — high contrast, never inherit dim border color.
pub fn panel_title() -> Style {
    Style::default()
        .fg(ACCENT)
        .bg(BG)
        .add_modifier(Modifier::BOLD)
}

pub fn panel_title_focused() -> Style {
    Style::default()
        .fg(BG)
        .bg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

pub fn normal() -> Style {
    Style::default().fg(FG).bg(BG)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED).bg(BG)
}

pub fn selected() -> Style {
    Style::default()
        .fg(BG)
        .bg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

pub fn border() -> Style {
    Style::default().fg(BORDER).bg(BG)
}

pub fn border_focused() -> Style {
    Style::default().fg(ACCENT).bg(BG)
}

pub fn status_ok() -> Style {
    Style::default().fg(OK).bg(BG)
}

pub fn status_err() -> Style {
    Style::default().fg(ERR).bg(BG)
}

pub fn status_changed() -> Style {
    Style::default()
        .fg(CHANGED)
        .bg(BG)
        .add_modifier(Modifier::BOLD)
}

pub fn status_waiting() -> Style {
    Style::default().fg(MUTED).bg(BG)
}
