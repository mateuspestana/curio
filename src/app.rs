//! Application state and TUI event loop.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::db::Database;
use crate::interval::{format_interval, parse_interval};
use crate::model::{AppEvent, Change, Watch};
use crate::monitor::ScheduleMap;
use crate::ui::{alerts, dashboard, form, history, theme, widgets};
use crate::ui::form::{FormField, FormState};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::CrosstermBackend,
    widgets::{Block, Borders, Clear, Paragraph},
    Terminal,
};
use std::collections::HashMap;
use std::io::Stdout;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Dashboard,
    Form,
    History,
    Alerts,
    Help,
    ConfirmDelete,
}

pub struct App {
    db_path: PathBuf,
    watches: Vec<Watch>,
    selected: usize,
    screen: Screen,
    form: FormState,
    changes: Vec<Change>,
    change_selected: usize,
    diff_scroll: u16,
    alerts: Vec<(Change, String, String)>,
    flash: Vec<String>,
    schedule: ScheduleMap,
    reload: Arc<Mutex<bool>>,
    events: mpsc::UnboundedReceiver<AppEvent>,
    should_quit: bool,
}

impl App {
    pub fn new(
        db_path: PathBuf,
        schedule: ScheduleMap,
        reload: Arc<Mutex<bool>>,
        events: mpsc::UnboundedReceiver<AppEvent>,
    ) -> Result<Self> {
        let mut app = Self {
            db_path,
            watches: Vec::new(),
            selected: 0,
            screen: Screen::Dashboard,
            form: FormState::new_add(),
            changes: Vec::new(),
            change_selected: 0,
            diff_scroll: 0,
            alerts: Vec::new(),
            flash: Vec::new(),
            schedule,
            reload,
            events,
            should_quit: false,
        };
        app.reload_watches()?;
        app.reload_alerts()?;
        Ok(app)
    }

    fn db(&self) -> Result<Database> {
        Database::open_path(&self.db_path)
    }

    fn reload_watches(&mut self) -> Result<()> {
        self.watches = self.db()?.list_watches()?;
        if self.selected >= self.watches.len() && !self.watches.is_empty() {
            self.selected = self.watches.len() - 1;
        }
        Ok(())
    }

    fn reload_alerts(&mut self) -> Result<()> {
        self.alerts = self.db()?.list_recent_changes(50)?;
        Ok(())
    }

    async fn request_reload(&self) {
        *self.reload.lock().await = true;
    }

    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        while !self.should_quit {
            while let Ok(ev) = self.events.try_recv() {
                self.handle_app_event(ev)?;
            }

            let schedule_snapshot = self.schedule.lock().await.clone();
            terminal.draw(|f| self.draw(f, &schedule_snapshot))?;

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key.code, key.modifiers).await?;
                    }
                }
            }
        }
        Ok(())
    }

    fn handle_app_event(&mut self, ev: AppEvent) -> Result<()> {
        match ev {
            AppEvent::WatchUpdated(_) | AppEvent::Tick => {
                self.reload_watches()?;
            }
            AppEvent::ChangeDetected { label, .. } => {
                self.flash
                    .push(format!("Mudança detectada: {label}"));
                if self.flash.len() > 30 {
                    self.flash.remove(0);
                }
                self.reload_watches()?;
                self.reload_alerts()?;
            }
            AppEvent::Downtime { label, error, .. } => {
                self.flash
                    .push(format!("Indisponível: {label} — {error}"));
                if self.flash.len() > 30 {
                    self.flash.remove(0);
                }
                self.reload_watches()?;
            }
        }
        Ok(())
    }

    fn draw(&self, frame: &mut ratatui::Frame, schedule: &HashMap<i64, Instant>) {
        let area = frame.area();
        // Clear with background
        let bg = Block::default().style(theme::normal());
        frame.render_widget(bg, area);

        let (header, body, footer) = widgets::main_chunks(area);
        let subtitle = match self.screen {
            Screen::Dashboard => "monitoramento de URLs",
            Screen::Form => "formulário",
            Screen::History => "histórico",
            Screen::Alerts => "alertas",
            Screen::Help => "ajuda",
            Screen::ConfirmDelete => "confirmar exclusão",
        };
        widgets::draw_header(frame, header, subtitle);

        match self.screen {
            Screen::Dashboard | Screen::ConfirmDelete => {
                dashboard::draw(frame, body, &self.watches, self.selected, schedule);
                if self.screen == Screen::ConfirmDelete {
                    draw_modal(
                        frame,
                        body,
                        "Excluir este monitor? [s] sim  [n] não",
                    );
                }
            }
            Screen::Form => form::draw(frame, body, &self.form),
            Screen::History => {
                let title = self
                    .watches
                    .get(self.selected)
                    .map(|w| {
                        if w.label.is_empty() {
                            w.url.clone()
                        } else {
                            w.label.clone()
                        }
                    })
                    .unwrap_or_default();
                history::draw(
                    frame,
                    body,
                    &title,
                    &self.changes,
                    self.change_selected,
                    self.diff_scroll,
                );
            }
            Screen::Alerts => alerts::draw(frame, body, &self.alerts, &self.flash),
            Screen::Help => draw_help(frame, body),
        }

        let hints = match self.screen {
            Screen::Dashboard => {
                "a adicionar · e editar · d excluir · o abrir · h histórico · l alertas · ? ajuda · q sair"
            }
            Screen::Form => "Tab campo · Enter salvar · Esc cancelar",
            Screen::History => {
                "↑↓ mudança · PgUp/PgDn rolar · m marcar visto · Esc voltar · o abrir"
            }
            Screen::Alerts => "m marcar todos vistos · Esc voltar",
            Screen::Help | Screen::ConfirmDelete => "Esc voltar",
        };
        widgets::draw_footer(frame, footer, hints);
    }

    async fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            self.should_quit = true;
            return Ok(());
        }

        match self.screen {
            Screen::Dashboard => self.key_dashboard(code).await?,
            Screen::Form => self.key_form(code).await?,
            Screen::History => self.key_history(code).await?,
            Screen::Alerts => self.key_alerts(code)?,
            Screen::Help => {
                if matches!(code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')) {
                    self.screen = Screen::Dashboard;
                }
            }
            Screen::ConfirmDelete => self.key_confirm_delete(code).await?,
        }
        Ok(())
    }

    async fn key_dashboard(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.screen = Screen::Help,
            KeyCode::Char('a') => {
                self.form = FormState::new_add();
                self.screen = Screen::Form;
            }
            KeyCode::Char('e') => {
                if let Some(w) = self.watches.get(self.selected).cloned() {
                    self.form = FormState::new_edit(
                        w.id,
                        w.url,
                        w.label,
                        format_interval(w.interval_secs),
                    );
                    self.screen = Screen::Form;
                }
            }
            KeyCode::Char('d') => {
                if !self.watches.is_empty() {
                    self.screen = Screen::ConfirmDelete;
                }
            }
            KeyCode::Char('o') => {
                if let Some(w) = self.watches.get(self.selected) {
                    let _ = open::that(&w.url);
                }
            }
            KeyCode::Char('h') => {
                if let Some(w) = self.watches.get(self.selected) {
                    self.changes = self.db()?.list_changes(w.id, 100)?;
                    self.change_selected = 0;
                    self.diff_scroll = 0;
                    self.screen = Screen::History;
                }
            }
            KeyCode::Char('l') => {
                self.reload_alerts()?;
                self.screen = Screen::Alerts;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.watches.is_empty() {
                    self.selected = (self.selected + 1) % self.watches.len();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.watches.is_empty() {
                    self.selected = if self.selected == 0 {
                        self.watches.len() - 1
                    } else {
                        self.selected - 1
                    };
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn key_form(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Esc => self.screen = Screen::Dashboard,
            KeyCode::Tab | KeyCode::Down => self.form.next_field(),
            KeyCode::BackTab | KeyCode::Up => self.form.prev_field(),
            KeyCode::Enter => self.submit_form().await?,
            KeyCode::Backspace => {
                self.form.focused_mut().pop();
            }
            KeyCode::Char(c) => {
                // Ignore control-ish chars
                if !c.is_control() {
                    self.form.focused_mut().push(c);
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn submit_form(&mut self) -> Result<()> {
        self.form.error = None;
        let url = self.form.url.trim().to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            self.form.error = Some("URL deve começar com http:// ou https://".into());
            self.form.focus = FormField::Url;
            return Ok(());
        }
        let interval = match parse_interval(&self.form.interval) {
            Ok(v) => v,
            Err(e) => {
                self.form.error = Some(e.to_string());
                self.form.focus = FormField::Interval;
                return Ok(());
            }
        };
        let label = self.form.label.trim().to_string();
        let db = self.db()?;
        if let Some(id) = self.form.editing_id {
            db.update_watch(id, &url, &label, interval)?;
        } else {
            db.insert_watch(&url, &label, interval)?;
        }
        self.reload_watches()?;
        self.request_reload().await;
        self.screen = Screen::Dashboard;
        Ok(())
    }

    async fn key_history(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Esc => self.screen = Screen::Dashboard,
            KeyCode::Char('o') => {
                if let Some(w) = self.watches.get(self.selected) {
                    let _ = open::that(&w.url);
                }
            }
            KeyCode::Char('m') => {
                if let Some(w) = self.watches.get(self.selected) {
                    self.db()?.mark_changes_seen_for_watch(w.id)?;
                    self.changes = self.db()?.list_changes(w.id, 100)?;
                    self.reload_watches()?;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.changes.is_empty() {
                    self.change_selected = (self.change_selected + 1) % self.changes.len();
                    self.diff_scroll = 0;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.changes.is_empty() {
                    self.change_selected = if self.change_selected == 0 {
                        self.changes.len() - 1
                    } else {
                        self.change_selected - 1
                    };
                    self.diff_scroll = 0;
                }
            }
            KeyCode::PageDown => self.diff_scroll = self.diff_scroll.saturating_add(5),
            KeyCode::PageUp => self.diff_scroll = self.diff_scroll.saturating_sub(5),
            _ => {}
        }
        Ok(())
    }

    fn key_alerts(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Esc => self.screen = Screen::Dashboard,
            KeyCode::Char('m') => {
                self.db()?.mark_all_changes_seen()?;
                self.reload_alerts()?;
                self.reload_watches()?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn key_confirm_delete(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(w) = self.watches.get(self.selected) {
                    self.db()?.delete_watch(w.id)?;
                    self.reload_watches()?;
                    self.request_reload().await;
                }
                self.screen = Screen::Dashboard;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.screen = Screen::Dashboard;
            }
            _ => {}
        }
        Ok(())
    }
}

fn draw_help(frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    let text = "\
Curió — atalhos

  a          adicionar monitor
  e          editar selecionado
  d          excluir selecionado
  o          abrir URL no navegador
  h          histórico / diferenças
  l          painel de alertas
  m          marcar como visto
  ?          esta ajuda
  q          sair

Detecção: texto visível normalizado.
Falhas de rede: até 3 tentativas (intervalo de 5 min entre novas tentativas).
";
    let p = Paragraph::new(text).style(theme::normal()).block(
        Block::default()
            .title(" Ajuda ")
            .title_style(theme::panel_title())
            .borders(Borders::ALL)
            .border_style(theme::border()),
    );
    frame.render_widget(p, area);
}

fn draw_modal(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, msg: &str) {
    let popup = centered_rect(60, 5, area);
    frame.render_widget(Clear, popup);
    let p = Paragraph::new(msg).style(theme::selected()).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(ratatui::style::Style::default().fg(theme::ACCENT)),
    );
    frame.render_widget(p, popup);
}

fn centered_rect(
    percent_x: u16,
    height: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    use ratatui::layout::{Constraint, Direction, Layout};
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height.min(100)) / 2),
            Constraint::Length(height),
            Constraint::Percentage((100 - height.min(100)) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
