//! Dashboard list of watches.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::interval::format_interval;
use crate::model::{Watch, WatchStatus};
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
) {
    let items: Vec<ListItem> = watches
        .iter()
        .enumerate()
        .map(|(_i, w)| {
            let name = if w.label.is_empty() {
                truncate(&w.url, 40)
            } else {
                truncate(&w.label, 28)
            };
            let unread = if w.unread_change { " ●" } else { "  " };
            let status = status_label(w.last_status);
            let status_style = status_style(w.last_status);
            let interval = format_interval(w.interval_secs);
            let eta = countdown(schedule.get(&w.id).copied());
            let last = w
                .last_checked_at
                .map(|t| t.with_timezone(&Utc).format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "—".into());
            let err = w
                .last_error
                .as_deref()
                .filter(|_| w.last_status == WatchStatus::Error)
                .map(|e| format!(" · {e}"))
                .unwrap_or_default();

            let line = Line::from(vec![
                Span::styled(unread, theme::status_changed()),
                Span::styled(format!("{name:<30} "), theme::normal()),
                Span::styled(format!("{status:<8} "), status_style),
                Span::styled(format!("{interval:<6} "), theme::muted()),
                Span::styled(format!("próx {eta:<8} "), theme::muted()),
                Span::styled(format!("último {last}{err}"), theme::muted()),
            ]);
            ListItem::new(line)
        })
        .collect();

    let mut state = ListState::default();
    if !watches.is_empty() {
        state.select(Some(selected.min(watches.len() - 1)));
    }

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Monitores ")
                .title_style(theme::panel_title())
                .borders(Borders::ALL)
                .border_style(theme::border())
                .style(theme::normal()),
        )
        .highlight_style(theme::selected())
        .highlight_symbol("› ");

    frame.render_stateful_widget(list, area, &mut state);
}

fn status_label(s: WatchStatus) -> &'static str {
    match s {
        WatchStatus::Waiting => "aguardando",
        WatchStatus::Ok => "ok",
        WatchStatus::Changed => "mudou",
        WatchStatus::Error => "erro",
    }
}

fn status_style(s: WatchStatus) -> Style {
    match s {
        WatchStatus::Waiting => theme::status_waiting(),
        WatchStatus::Ok => theme::status_ok(),
        WatchStatus::Changed => theme::status_changed(),
        WatchStatus::Error => theme::status_err(),
    }
}

fn countdown(target: Option<Instant>) -> String {
    let Some(t) = target else {
        return "—".into();
    };
    let d = t.saturating_duration_since(Instant::now());
    format_duration(d)
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
