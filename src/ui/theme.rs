//! Visual theme for Curió (dark/light + amber accent).
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::model::ThemeMode;
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color,
    pub fg: Color,
    pub muted: Color,
    pub accent: Color,
    pub ok: Color,
    pub err: Color,
    pub changed: Color,
    pub border: Color,
    pub unread_bg: Color,
}

impl Palette {
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self {
                bg: Color::Rgb(14, 16, 20),
                fg: Color::Rgb(240, 242, 245),
                muted: Color::Rgb(168, 176, 188),
                accent: Color::Rgb(242, 180, 64),
                ok: Color::Rgb(96, 200, 130),
                err: Color::Rgb(235, 110, 110),
                changed: Color::Rgb(110, 200, 215),
                border: Color::Rgb(110, 120, 135),
                unread_bg: Color::Rgb(48, 36, 12),
            },
            ThemeMode::Light => Self {
                bg: Color::Rgb(248, 249, 251),
                fg: Color::Rgb(22, 26, 32),
                muted: Color::Rgb(90, 98, 110),
                accent: Color::Rgb(180, 110, 10),
                ok: Color::Rgb(20, 130, 70),
                err: Color::Rgb(180, 40, 40),
                changed: Color::Rgb(10, 120, 140),
                border: Color::Rgb(150, 158, 170),
                unread_bg: Color::Rgb(255, 236, 200),
            },
        }
    }
}

thread_local! {
    static CURRENT: std::cell::Cell<ThemeMode> = const { std::cell::Cell::new(ThemeMode::Dark) };
}

pub fn set_mode(mode: ThemeMode) {
    CURRENT.with(|c| c.set(mode));
}

pub fn mode() -> ThemeMode {
    CURRENT.with(|c| c.get())
}

fn p() -> Palette {
    Palette::for_mode(mode())
}

pub fn title() -> Style {
    let p = p();
    Style::default()
        .fg(p.accent)
        .bg(p.bg)
        .add_modifier(Modifier::BOLD)
}

pub fn panel_title() -> Style {
    title()
}

pub fn panel_title_focused() -> Style {
    let p = p();
    Style::default()
        .fg(p.bg)
        .bg(p.accent)
        .add_modifier(Modifier::BOLD)
}

pub fn normal() -> Style {
    let p = p();
    Style::default().fg(p.fg).bg(p.bg)
}

pub fn muted() -> Style {
    let p = p();
    Style::default().fg(p.muted).bg(p.bg)
}

pub fn selected() -> Style {
    let p = p();
    Style::default()
        .fg(p.bg)
        .bg(p.accent)
        .add_modifier(Modifier::BOLD)
}

pub fn border() -> Style {
    let p = p();
    Style::default().fg(p.border).bg(p.bg)
}

pub fn border_focused() -> Style {
    let p = p();
    Style::default().fg(p.accent).bg(p.bg)
}

pub fn status_ok() -> Style {
    let p = p();
    Style::default().fg(p.ok).bg(p.bg)
}

pub fn status_err() -> Style {
    let p = p();
    Style::default().fg(p.err).bg(p.bg)
}

pub fn status_changed() -> Style {
    let p = p();
    Style::default()
        .fg(p.changed)
        .bg(p.bg)
        .add_modifier(Modifier::BOLD)
}

pub fn status_waiting() -> Style {
    muted()
}

pub fn status_paused() -> Style {
    muted()
}

pub fn unread_row() -> Style {
    let p = p();
    Style::default().fg(p.fg).bg(p.unread_bg)
}

pub fn accent_fg() -> Color {
    p().accent
}
