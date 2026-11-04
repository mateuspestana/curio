//! Add / edit watch form with selector, ignore regex and preview.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::ui::theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Url,
    Label,
    Interval,
    Selector,
    IgnoreRegex,
}

#[derive(Debug, Clone)]
pub struct FormState {
    pub editing_id: Option<i64>,
    pub url: String,
    pub label: String,
    pub interval: String,
    pub selector: String,
    pub ignore_regex: String,
    pub focus: FormField,
    pub error: Option<String>,
    pub preview: Option<String>,
    pub preview_scroll: u16,
}

impl FormState {
    pub fn new_add() -> Self {
        Self {
            editing_id: None,
            url: String::new(),
            label: String::new(),
            interval: "10m".into(),
            selector: String::new(),
            ignore_regex: String::new(),
            focus: FormField::Url,
            error: None,
            preview: None,
            preview_scroll: 0,
        }
    }

    pub fn new_edit(
        id: i64,
        url: String,
        label: String,
        interval: String,
        selector: String,
        ignore_regex: String,
    ) -> Self {
        Self {
            editing_id: Some(id),
            url,
            label,
            interval,
            selector,
            ignore_regex,
            focus: FormField::Url,
            error: None,
            preview: None,
            preview_scroll: 0,
        }
    }

    pub fn focused_mut(&mut self) -> &mut String {
        match self.focus {
            FormField::Url => &mut self.url,
            FormField::Label => &mut self.label,
            FormField::Interval => &mut self.interval,
            FormField::Selector => &mut self.selector,
            FormField::IgnoreRegex => &mut self.ignore_regex,
        }
    }

    pub fn next_field(&mut self) {
        self.focus = match self.focus {
            FormField::Url => FormField::Label,
            FormField::Label => FormField::Interval,
            FormField::Interval => FormField::Selector,
            FormField::Selector => FormField::IgnoreRegex,
            FormField::IgnoreRegex => FormField::Url,
        };
    }

    pub fn prev_field(&mut self) {
        self.focus = match self.focus {
            FormField::Url => FormField::IgnoreRegex,
            FormField::Label => FormField::Url,
            FormField::Interval => FormField::Label,
            FormField::Selector => FormField::Interval,
            FormField::IgnoreRegex => FormField::Selector,
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
    draw_field(
        frame,
        chunks[3],
        "Seletor CSS/XPath (opcional)",
        &form.selector,
        form.focus == FormField::Selector,
    );
    draw_field(
        frame,
        chunks[4],
        "Ignorar regex (1 por linha)",
        &form.ignore_regex,
        form.focus == FormField::IgnoreRegex,
    );

    let err = form.error.clone().unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Span::styled(err, theme::status_err())),
        chunks[5],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Tab campos · F5/v prévia · Enter salvar · Esc cancelar",
            theme::muted(),
        ))),
        chunks[6],
    );

    if let Some(preview) = &form.preview {
        draw_preview(frame, area, preview, form.preview_scroll);
    }
}

fn draw_preview(frame: &mut Frame, area: Rect, text: &str, scroll: u16) {
    let popup = centered(area, 80, 70);
    frame.render_widget(Clear, popup);
    let p = Paragraph::new(text.to_string())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0))
        .style(theme::normal())
        .block(
            Block::default()
                .title(" Prévia do texto normalizado (Esc fecha · PgUp/PgDn) ")
                .title_style(theme::panel_title())
                .borders(Borders::ALL)
                .border_style(theme::border_focused()),
        );
    frame.render_widget(p, popup);
}

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(vert[1])[1]
}

fn draw_field(frame: &mut Frame, area: Rect, label: &str, value: &str, focused: bool) {
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
    let display = if value.chars().count() > 80 {
        let t: String = value.chars().take(77).collect();
        format!("{t}…█")
    } else {
        format!("{value}█")
    };
    let p = Paragraph::new(display).style(content_style).block(
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
