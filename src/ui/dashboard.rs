//! Dashboard list of watches.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::interval::format_interval;
use crate::model::{Density, Mode, Watch, WatchStatus};
use crate::ui::theme;
use chrono::Local;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::Instant;
use unicode_width::UnicodeWidthStr;

/// Tudo que o painel precisa para desenhar a lista de monitores.
#[derive(Clone, Copy)]
pub struct DashboardView<'a> {
    pub watches: &'a [Watch],
    pub selected: usize,
    pub schedule: &'a HashMap<i64, Instant>,
    pub density: Density,
    pub unread_total: u64,
    pub filter_note: &'a str,
}

pub fn draw(frame: &mut Frame, area: Rect, view: DashboardView) {
    let DashboardView {
        watches,
        selected,
        schedule,
        density,
        unread_total,
        filter_note,
    } = view;
    let compact = density == Density::Compact;
    // Colunas fixas: o status alinha mesmo quando há tags e modo na linha.
    let name_col = if compact { 28 } else { 36 };
    let tags_col = match (watches.iter().any(|w| !w.tags.is_empty()), compact) {
        (false, _) => 0,
        (true, true) => 12,
        (true, false) => 14,
    };
    let items: Vec<ListItem> = watches
        .iter()
        .map(|w| {
            let name = if w.label.is_empty() {
                truncate(&w.url, if compact { 28 } else { 36 })
            } else {
                truncate(&w.label, if compact { 20 } else { 28 })
            };
            let badge = if w.unread_change {
                " ● NOVO "
            } else {
                "        "
            };
            let paused = if w.paused { "⏸ " } else { "  " };
            let status = status_label(w);
            let status_style = status_style(w.last_status, w.paused);
            let interval = format_interval(w.interval_secs);
            let eta = if w.paused {
                "pausado".into()
            } else {
                countdown(schedule.get(&w.id).copied())
            };
            let last = w
                .last_checked_at
                .map(|t| t.with_timezone(&Local).format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "—".into());
            let sel = if w.selector.is_empty() {
                String::new()
            } else {
                format!(" [{}]", truncate(&w.selector, 12))
            };
            let mode_tag = match w.mode {
                Mode::Html => String::new(),
                m => format!(" ‹{}›", m.as_str()),
            };
            let tags: String = w.tag_list().map(|t| format!(" #{t}")).collect();
            let fails = if w.consecutive_failures > 0 {
                format!(" ×{}", w.consecutive_failures)
            } else {
                String::new()
            };
            let err = w
                .last_error
                .as_deref()
                .filter(|_| w.last_status == WatchStatus::Error && !w.paused)
                .map(|e| format!("{fails} · {e}"))
                .unwrap_or_default();

            let mut spans = vec![
                Span::styled(badge, theme::status_changed()),
                Span::styled(paused, theme::status_paused()),
                Span::styled(
                    pad_to(&format!("{name}{sel}{mode_tag}"), name_col),
                    theme::normal(),
                ),
                Span::styled(pad_to(&tags, tags_col), theme::status_waiting()),
            ];
            if compact {
                spans.push(Span::styled(
                    format!("{status} {interval} {eta}"),
                    status_style,
                ));
            } else {
                spans.extend([
                    Span::styled(format!("{status:<9} "), status_style),
                    Span::styled(format!("{interval:<6} "), theme::muted()),
                    Span::styled(format!("próx {eta:<8} "), theme::muted()),
                    Span::styled(format!("último {last}{err}"), theme::muted()),
                ]);
            }

            let line = Line::from(spans);
            let item = ListItem::new(line);
            if w.unread_change {
                item.style(theme::unread_row())
            } else {
                item
            }
        })
        .collect();

    let mut state = ListState::default();
    if !watches.is_empty() {
        state.select(Some(selected.min(watches.len() - 1)));
    }

    let title = match (unread_total > 0, filter_note.is_empty()) {
        (true, true) => format!(" Monitores  ·  {unread_total} não lido(s) "),
        (true, false) => format!(" Monitores  ·  {unread_total} não lido(s)  ·  {filter_note} "),
        (false, false) => format!(" Monitores  ·  {filter_note} "),
        (false, true) => " Monitores ".into(),
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .title_style(theme::panel_title())
                .borders(Borders::ALL)
                .border_style(theme::border())
                .style(theme::normal()),
        )
        .highlight_style(theme::selected())
        .highlight_symbol("› ");

    frame.render_stateful_widget(list, area, &mut state);
}

fn status_label(w: &Watch) -> &'static str {
    if w.paused {
        return "pausado";
    }
    match w.last_status {
        WatchStatus::Waiting => "aguardando",
        WatchStatus::Ok => "ok",
        WatchStatus::Changed => "mudou",
        WatchStatus::Error => "erro",
        WatchStatus::Paused => "pausado",
    }
}

fn status_style(s: WatchStatus, paused: bool) -> Style {
    if paused {
        return theme::status_paused();
    }
    match s {
        WatchStatus::Waiting => theme::status_waiting(),
        WatchStatus::Ok => theme::status_ok(),
        WatchStatus::Changed => theme::status_changed(),
        WatchStatus::Error => theme::status_err(),
        WatchStatus::Paused => theme::status_paused(),
    }
}

fn countdown(target: Option<Instant>) -> String {
    let Some(t) = target else {
        return "—".into();
    };
    format_duration(t.saturating_duration_since(Instant::now()))
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// Corta (com `…`) e completa com espaços até exatamente `width` colunas, sempre
/// deixando ao menos um espaço no fim como separador (0 = sem coluna).
fn pad_to(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let inner = width - 1;
    let mut out = String::new();
    let mut w = 0;
    if s.width() <= inner {
        out.push_str(s);
        w = s.width();
    } else {
        for c in s.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if w + cw > inner.saturating_sub(1) {
                break;
            }
            out.push(c);
            w += cw;
        }
        out.push('…');
        w += 1;
    }
    format!("{out}{}", " ".repeat(width - w))
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_to_makes_fixed_width_columns() {
        assert_eq!(pad_to("abc", 6), "abc   ");
        assert_eq!(pad_to("abc", 6).width(), 6);
        assert_eq!(pad_to("", 0), "");
        let cut = pad_to("um nome bem comprido demais", 10);
        assert_eq!(cut.width(), 10);
        assert!(cut.contains('…'));
        assert_eq!(pad_to("ação", 8).width(), 8);
    }
}
