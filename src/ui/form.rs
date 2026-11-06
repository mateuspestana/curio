//! Add / edit watch form: modo, seletor, filtros, condição, cabeçalhos, tags e prévia.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::interval::{format_interval, parse_interval};
use crate::model::Mode;
use crate::spec::{SpecField, WatchSpec};
use crate::ui::theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Url,
    Label,
    Interval,
    Mode,
    Selector,
    IgnoreRegex,
    Condition,
    Headers,
    Tags,
}

impl FormField {
    pub const ALL: [FormField; 9] = [
        FormField::Url,
        FormField::Label,
        FormField::Interval,
        FormField::Mode,
        FormField::Selector,
        FormField::IgnoreRegex,
        FormField::Condition,
        FormField::Headers,
        FormField::Tags,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    /// Campos que aceitam várias linhas (Ctrl+N insere a quebra).
    pub fn is_multiline(self) -> bool {
        matches!(self, Self::IgnoreRegex | Self::Condition | Self::Headers)
    }
}

impl From<SpecField> for FormField {
    fn from(f: SpecField) -> Self {
        match f {
            SpecField::Url => Self::Url,
            SpecField::Interval => Self::Interval,
            SpecField::Selector => Self::Selector,
            SpecField::Ignore => Self::IgnoreRegex,
            SpecField::Condition => Self::Condition,
            SpecField::Headers => Self::Headers,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FormState {
    pub editing_id: Option<i64>,
    pub url: String,
    pub label: String,
    pub interval: String,
    pub mode: Mode,
    pub selector: String,
    pub ignore_regex: String,
    pub condition: String,
    pub headers: String,
    pub tags: String,
    pub focus: FormField,
    pub error: Option<String>,
    pub preview: Option<String>,
    pub preview_scroll: u16,
}

impl FormState {
    pub fn new_add() -> Self {
        Self::from_spec(
            None,
            &WatchSpec {
                interval_secs: 600,
                ..Default::default()
            },
        )
    }

    pub fn from_spec(editing_id: Option<i64>, spec: &WatchSpec) -> Self {
        Self {
            editing_id,
            url: spec.url.clone(),
            label: spec.label.clone(),
            interval: format_interval(spec.interval_secs),
            mode: spec.mode,
            selector: spec.selector.clone(),
            ignore_regex: spec.ignore_regex.clone(),
            condition: spec.condition.clone(),
            headers: spec.headers.clone(),
            tags: spec.tags.replace(',', ", "),
            focus: FormField::Url,
            error: None,
            preview: None,
            preview_scroll: 0,
        }
    }

    /// Converte os campos em uma [`WatchSpec`] normalizada (sem validar o conteúdo).
    pub fn to_spec(&self) -> Result<WatchSpec, (FormField, String)> {
        let interval_secs =
            parse_interval(&self.interval).map_err(|e| (FormField::Interval, e.to_string()))?;
        Ok(WatchSpec {
            url: self.url.clone(),
            label: self.label.clone(),
            interval_secs,
            mode: self.mode,
            selector: self.selector.clone(),
            ignore_regex: self.ignore_regex.clone(),
            condition: self.condition.clone(),
            headers: self.headers.clone(),
            tags: self.tags.clone(),
        }
        .normalized())
    }

    /// Texto do campo em foco (`None` para o seletor de modo).
    pub fn focused_mut(&mut self) -> Option<&mut String> {
        Some(match self.focus {
            FormField::Url => &mut self.url,
            FormField::Label => &mut self.label,
            FormField::Interval => &mut self.interval,
            FormField::Mode => return None,
            FormField::Selector => &mut self.selector,
            FormField::IgnoreRegex => &mut self.ignore_regex,
            FormField::Condition => &mut self.condition,
            FormField::Headers => &mut self.headers,
            FormField::Tags => &mut self.tags,
        })
    }

    pub fn next_field(&mut self) {
        self.focus = FormField::ALL[(self.focus.index() + 1) % FormField::ALL.len()];
    }

    pub fn prev_field(&mut self) {
        let n = FormField::ALL.len();
        self.focus = FormField::ALL[(self.focus.index() + n - 1) % n];
    }
}

/// Quantos campos (3 linhas cada) cabem; sobra espaço para erro e dica.
fn visible_capacity(area: Rect) -> usize {
    // borda (2) + margem (2) + linha de erro (1) + linha de dica (1)
    ((area.height.saturating_sub(6)) / 3).max(1) as usize
}

pub fn draw(frame: &mut Frame, area: Rect, form: &FormState) {
    let capacity = visible_capacity(area).min(FormField::ALL.len());
    let focus_idx = form.focus.index();
    // A janela acompanha o foco: com terminais baixos o formulário rola.
    let start = if focus_idx < capacity {
        0
    } else {
        focus_idx + 1 - capacity
    };
    let shown = &FormField::ALL[start..start + capacity];

    let kind = if form.editing_id.is_some() {
        "Editar monitor"
    } else {
        "Novo monitor"
    };
    let title = format!(
        " {kind} · campo {}/{} ",
        focus_idx + 1,
        FormField::ALL.len()
    );

    let mut constraints: Vec<Constraint> = shown.iter().map(|_| Constraint::Length(3)).collect();
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Min(1));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(area);

    let block = Block::default()
        .title(title)
        .title_style(theme::panel_title())
        .borders(Borders::ALL)
        .border_style(theme::border())
        .style(theme::normal());
    frame.render_widget(block, area);

    for (i, field) in shown.iter().enumerate() {
        let (label, value) = field_view(form, *field);
        draw_field(frame, chunks[i], &label, &value, form.focus == *field);
    }

    let err = form.error.clone().unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Span::styled(err, theme::status_err())),
        chunks[shown.len()],
    );

    let hint = match form.focus {
        FormField::Mode => {
            "←/→ ou espaço muda o modo · Tab campos · F5 prévia · Enter salva · Esc cancela"
        }
        f if f.is_multiline() => {
            "Ctrl+N nova linha · Tab campos · F5 prévia · Enter salva · Esc cancela"
        }
        _ => "Tab campos · F5 prévia · Enter salva · Esc cancela",
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, theme::muted()))),
        chunks[shown.len() + 1],
    );

    if let Some(preview) = &form.preview {
        draw_preview(frame, area, preview, form.preview_scroll);
    }
}

/// Rótulo e texto exibido de cada campo (quebras de linha viram `↵`).
fn field_view(form: &FormState, field: FormField) -> (String, String) {
    let one_line = |s: &str| s.replace('\n', " ↵ ");
    match field {
        FormField::Url => ("URL".into(), form.url.clone()),
        FormField::Label => ("Rótulo (opcional)".into(), form.label.clone()),
        FormField::Interval => (
            "Intervalo (ex.: 10s, 50m, 2d, 1w)".into(),
            form.interval.clone(),
        ),
        FormField::Mode => ("Modo".into(), form.mode.description().to_string()),
        FormField::Selector => (
            match form.mode {
                Mode::Json => "Caminho JSON (opcional, ex.: items[*].price)".into(),
                _ => "Seletor CSS/XPath (opcional)".into(),
            },
            form.selector.clone(),
        ),
        FormField::IgnoreRegex => (
            "Ignorar regex (uma por linha · Ctrl+N)".into(),
            one_line(&form.ignore_regex),
        ),
        FormField::Condition => (
            "Alertar só se… (contém: · não contém: · regex: · número < 100 · Ctrl+N)".into(),
            one_line(&form.condition),
        ),
        FormField::Headers => (
            "Cabeçalhos HTTP (Nome: valor · use ${VAR} para segredos · Ctrl+N)".into(),
            one_line(&form.headers),
        ),
        FormField::Tags => ("Tags (separadas por vírgula)".into(), form.tags.clone()),
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

/// Mostra o final do texto (onde o cursor está) quando não cabe na largura.
fn tail_fit(value: &str, max_width: usize) -> String {
    if value.width() <= max_width {
        return value.to_string();
    }
    let mut out: Vec<char> = Vec::new();
    let mut w = 1; // reserva a reticência
    for c in value.chars().rev() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > max_width {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.reverse();
    format!("…{}", out.into_iter().collect::<String>())
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
    // Largura útil: borda (2) + cursor (1).
    let inner = (area.width as usize).saturating_sub(3);
    let display = format!(
        "{}{}",
        tail_fit(value, inner),
        if focused { "█" } else { "" }
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_navigation_wraps_around() {
        let mut f = FormState::new_add();
        for _ in 0..FormField::ALL.len() {
            f.next_field();
        }
        assert_eq!(f.focus, FormField::Url);
        f.prev_field();
        assert_eq!(f.focus, FormField::Tags);
    }

    #[test]
    fn spec_roundtrip_and_interval_error() {
        let spec = WatchSpec {
            url: "https://a.example".into(),
            interval_secs: 90,
            mode: Mode::Json,
            condition: "número < 5".into(),
            tags: "a,b".into(),
            ..Default::default()
        };
        let form = FormState::from_spec(Some(3), &spec);
        assert_eq!(form.interval, "90s");
        assert_eq!(form.tags, "a, b");
        assert_eq!(form.to_spec().unwrap(), spec);

        let mut bad = form.clone();
        bad.interval = "xx".into();
        assert_eq!(bad.to_spec().unwrap_err().0, FormField::Interval);
    }

    #[test]
    fn mode_field_has_no_text_buffer() {
        let mut f = FormState::new_add();
        f.focus = FormField::Mode;
        assert!(f.focused_mut().is_none());
        f.focus = FormField::Headers;
        assert!(f.focused_mut().is_some());
        assert!(FormField::Headers.is_multiline() && !FormField::Url.is_multiline());
    }

    #[test]
    fn tail_fit_keeps_the_cursor_end_visible() {
        assert_eq!(tail_fit("curto", 10), "curto");
        let t = tail_fit("abcdefghijklmnop", 6);
        assert_eq!(t, "…lmnop");
        assert!(t.width() <= 6);
    }

    #[test]
    fn window_follows_focus_on_short_terminals() {
        // 24 linhas de tela → corpo de ~18: cabem 4 campos.
        assert_eq!(visible_capacity(Rect::new(0, 0, 80, 18)), 4);
        assert_eq!(visible_capacity(Rect::new(0, 0, 80, 3)), 1);
    }
}
