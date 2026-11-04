//! Dashboard list of watches.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::interval::format_interval;
use crate::model::{Density, Watch, WatchStatus};
use crate::ui::theme;
use chrono::Utc;
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

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    watches: &[Watch],
    selected: usize,
    schedule: &HashMap<i64, Instant>,
    density: Density,
    unread_total: u64,
) {
    let compact = density == Density::Compact;
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
                .map(|t| t.with_timezone(&Utc).format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "—".into());
            let sel = if w.selector.is_empty() {
                String::new()
            } else {
                format!(" [{}]", truncate(&w.selector, 12))
            };
            let err = w
                .last_error
                .as_deref()
                .filter(|_| w.last_status == WatchStatus::Error && !w.paused)
                .map(|e| format!(" · {e}"))
                .unwrap_or_default();

            let mut spans = vec![
                Span::styled(badge, theme::status_changed()),
                Span::styled(paused, theme::status_paused()),
                Span::styled(format!("{name}{sel:<1} "), theme::normal()),
            ];
            if compact {
                spans.push(Span::styled(format!("{status} {interval} {eta}"), status_style));
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

    let title = if unread_total > 0 {
        format!(" Monitores  ·  {unread_total} não lido(s) ")
    } else {
        " Monitores ".into()
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
