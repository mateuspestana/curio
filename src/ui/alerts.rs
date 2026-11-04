//! Recent alerts feed.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::model::Change;
use crate::ui::theme;
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    alerts: &[(Change, String, String)],
    messages: &[String],
) {
    let mut items: Vec<ListItem> = messages
        .iter()
        .rev()
        .take(20)
        .map(|m| {
            ListItem::new(Line::from(Span::styled(m.clone(), theme::status_err())))
        })
        .collect();

    for (change, label, url) in alerts.iter().take(40) {
        let name = if label.is_empty() { url } else { label };
        let mark = if change.seen { " " } else { "●" };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("{mark} "), theme::status_changed()),
            Span::styled(
                format!("{}  ", change.detected_at.format("%H:%M:%S")),
                theme::muted(),
            ),
            Span::styled(format!("mudança · {name}"), theme::normal()),
        ])));
    }

    if items.is_empty() {
        items.push(ListItem::new(Span::styled(
            "Nenhum alerta ainda.",
            theme::muted(),
        )));
    }

    let list = List::new(items).block(
        Block::default()
            .title(" Alertas ")
            .title_style(theme::panel_title())
            .borders(Borders::ALL)
            .border_style(theme::border())
            .style(theme::normal()),
    );
    frame.render_widget(list, area);
}
