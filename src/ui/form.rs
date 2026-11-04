//! Add / edit watch form.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::ui::theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Url,
    Label,
    Interval,
}

#[derive(Debug, Clone)]
pub struct FormState {
    pub editing_id: Option<i64>,
    pub url: String,
    pub label: String,
    pub interval: String,
    pub focus: FormField,
    pub error: Option<String>,
}

impl FormState {
    pub fn new_add() -> Self {
        Self {
            editing_id: None,
            url: String::new(),
            label: String::new(),
            interval: "10m".into(),
            focus: FormField::Url,
            error: None,
        }
    }

    pub fn new_edit(id: i64, url: String, label: String, interval: String) -> Self {
        Self {
            editing_id: Some(id),
            url,
            label,
            interval,
            focus: FormField::Url,
            error: None,
        }
    }

    pub fn focused_mut(&mut self) -> &mut String {
        match self.focus {
            FormField::Url => &mut self.url,
            FormField::Label => &mut self.label,
            FormField::Interval => &mut self.interval,
        }
    }

    pub fn next_field(&mut self) {
        self.focus = match self.focus {
            FormField::Url => FormField::Label,
            FormField::Label => FormField::Interval,
            FormField::Interval => FormField::Url,
        };
    }

    pub fn prev_field(&mut self) {
        self.focus = match self.focus {
            FormField::Url => FormField::Interval,
            FormField::Label => FormField::Url,
            FormField::Interval => FormField::Label,
        };
    }
}

pub fn draw(frame: &mut Frame, area: Rect, form: &FormState) {
    let title = if form.editing_id.is_some() {
        " Editar monitor "
    } else {
        " Novo monitor "
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(area);

    let block = Block::default()
        .title(title)
        .title_style(theme::panel_title())
        .borders(Borders::ALL)
        .border_style(theme::border())
        .style(theme::normal());
    frame.render_widget(block, area);

    draw_field(frame, chunks[0], "URL", &form.url, form.focus == FormField::Url);
    draw_field(
        frame,
        chunks[1],
        "Rótulo (opcional)",
        &form.label,
        form.focus == FormField::Label,
    );
    draw_field(
        frame,
        chunks[2],
        "Intervalo (ex.: 10s, 50m, 2d, 1w)",
        &form.interval,
        form.focus == FormField::Interval,
    );

    let err = form.error.clone().unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Span::styled(err, theme::status_err())),
        chunks[3],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Tab/↑↓ campos · Enter salvar · Esc cancelar",
            theme::muted(),
        ))),
        chunks[4],
    );
}

fn draw_field(frame: &mut Frame, area: Rect, label: &str, value: &str, focused: bool) {
    // Conteúdo e título separados: o estilo do campo não pode escurecer o rótulo.
    let content_style = if focused {
        theme::selected()
    } else {
        theme::normal()
    };
    let title_style = if focused {
        theme::panel_title_focused()
    } else {
        theme::panel_title()
    };
    let p = Paragraph::new(format!("{value}█"))
        .style(content_style)
        .block(
            Block::default()
                .title(format!(" {label} "))
                .title_style(title_style)
                .borders(Borders::ALL)
                .border_style(if focused {
                    theme::border_focused()
                } else {
                    theme::border()
                })
                .style(theme::normal()),
        );
    frame.render_widget(p, area);
}
