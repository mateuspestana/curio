//! Application state and TUI event loop.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::db::Database;
use crate::extract::normalize_and_hash;
use crate::interval::{format_interval, parse_interval};
use crate::model::{AppEvent, AppSettings, Change, Watch};
use crate::monitor::{ForceCheckSet, ScheduleMap};
use crate::ui::form::{FormField, FormState};
use crate::ui::{alerts, dashboard, form, history, theme, widgets};
use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
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
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::time::Instant;

#[derive(Default)]
struct Reload {
    alerts: bool,
}

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
    db: Database,
    watches: Vec<Watch>,
    selected: usize,
    screen: Screen,
    form: FormState,
    changes: Vec<Change>,
    change_selected: usize,
    diff_scroll: u16,
    alerts: Vec<(Change, String, String)>,
    flash: Vec<String>,
    settings: AppSettings,
    unread_total: u64,
    schedule: ScheduleMap,
    reload: Arc<Mutex<bool>>,
    force: ForceCheckSet,
    wake: Arc<Notify>,
    events: mpsc::UnboundedReceiver<AppEvent>,
    should_quit: bool,
}

impl App {
    pub fn new(
        db_path: PathBuf,
        schedule: ScheduleMap,
        reload: Arc<Mutex<bool>>,
        force: ForceCheckSet,
        wake: Arc<Notify>,
        events: mpsc::UnboundedReceiver<AppEvent>,
    ) -> Result<Self> {
        let db = Database::open_path(&db_path)?;
        let settings = db.load_settings()?;
        theme::set_mode(settings.theme);
        let mut app = Self {
            db,
            watches: Vec::new(),
            selected: 0,
            screen: Screen::Dashboard,
            form: FormState::new_add(),
            changes: Vec::new(),
            change_selected: 0,
            diff_scroll: 0,
            alerts: Vec::new(),
            flash: Vec::new(),
            settings,
            unread_total: 0,
            schedule,
            reload,
            force,
            wake,
            events,
            should_quit: false,
        };
        app.reload_watches()?;
        app.reload_alerts()?;
        Ok(app)
    }

    fn db(&self) -> Result<&Database> {
        Ok(&self.db)
    }

    fn reload_watches(&mut self) -> Result<()> {
        self.watches = self.db.list_watches()?;
        self.unread_total = self.db.unread_count()?;
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
        self.wake.notify_one();
    }

    async fn request_force_check(&self, id: i64) {
        self.force.lock().await.insert(id);
        self.wake.notify_one();
    }

    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        let mut keys = EventStream::new();
        // 1 Hz: só para a contagem regressiva do dashboard.
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut dirty = true;
        let mut monitor_alive = true;

        while !self.should_quit {
            if dirty {
                let schedule_snapshot = self.schedule.lock().await.clone();
                terminal.draw(|f| self.draw(f, &schedule_snapshot))?;
                dirty = false;
            }

            tokio::select! {
                ev = keys.next() => match ev {
                    Some(Ok(Event::Key(key))) => {
                        if key.kind == KeyEventKind::Press {
                            self.handle_key(key.code, key.modifiers).await?;
                            dirty = true;
                        }
                    }
                    Some(Ok(Event::Resize(..))) => dirty = true,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                },
                ev = self.events.recv(), if monitor_alive => match ev {
                    Some(ev) => {
                        // Junta rajadas de eventos numa única recarga/redesenho.
                        let mut reload = Reload::default();
                        self.handle_app_event(ev, &mut reload);
                        while let Ok(ev) = self.events.try_recv() {
                            self.handle_app_event(ev, &mut reload);
                        }
                        self.reload_watches()?;
                        if reload.alerts {
                            self.reload_alerts()?;
                        }
                        dirty = true;
                    }
                    None => monitor_alive = false,
                },
                _ = ticker.tick() => {
                    if matches!(self.screen, Screen::Dashboard | Screen::ConfirmDelete) {
                        dirty = true;
                    }
                }
            }
        }
        Ok(())
    }

    fn handle_app_event(&mut self, ev: AppEvent, reload: &mut Reload) {
        match ev {
            AppEvent::WatchUpdated(_) => {}
            AppEvent::ChangeDetected { label, .. } => {
                self.push_flash(format!("Mudança detectada: {label}"));
                reload.alerts = true;
            }
            AppEvent::Downtime { label, error, .. } => {
                self.push_flash(format!("Indisponível: {label} — {error}"));
            }
        }
    }

    fn push_flash(&mut self, msg: String) {
        self.flash.push(msg);
        if self.flash.len() > 30 {
            self.flash.remove(0);
        }
    }

    fn draw(&self, frame: &mut ratatui::Frame, schedule: &HashMap<i64, Instant>) {
        let area = frame.area();
        frame.render_widget(Block::default().style(theme::normal()), area);

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
                dashboard::draw(
                    frame,
                    body,
                    &self.watches,
                    self.selected,
                    schedule,
                    self.settings.density,
                    self.unread_total,
                );
                if self.screen == Screen::ConfirmDelete {
                    draw_modal(frame, body, "Excluir este monitor? [s] sim  [n] não");
                }
            }
            Screen::Form => form::draw(frame, body, &self.form),
            Screen::History => {
                let title = self
                    .watches
                    .get(self.selected)
                    .map(|w| w.display_name().to_string())
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

        let theme_hint = match self.settings.theme {
            crate::model::ThemeMode::Dark => "escuro",
            crate::model::ThemeMode::Light => "claro",
        };
        let dens = match self.settings.density {
            crate::model::Density::Normal => "normal",
            crate::model::Density::Compact => "compacta",
        };

        let hints = match self.screen {
            Screen::Dashboard => format!(
                "a add · e edit · d del · p pausar · r checar · o abrir · h hist · l alertas · m lidos · t tema({theme_hint}) · c dens({dens}) · ? · q"
            ),
            Screen::Form => {
                "Tab campo · v prévia · Enter salvar · Esc cancelar".into()
            }
            Screen::History => {
                "↑↓ · PgUp/PgDn · m visto · Esc · o abrir".into()
            }
            Screen::Alerts => "m marcar todos vistos · Esc voltar".into(),
            Screen::Help | Screen::ConfirmDelete => "Esc voltar".into(),
        };
        widgets::draw_footer(frame, footer, &hints);
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
                        w.selector,
                        w.ignore_regex,
                    );
                    self.screen = Screen::Form;
                }
            }
            KeyCode::Char('d') => {
                if !self.watches.is_empty() {
                    self.screen = Screen::ConfirmDelete;
                }
            }
            KeyCode::Char('p') => {
                let pause = self
                    .watches
                    .get(self.selected)
                    .map(|w| (w.id, w.display_name().to_string(), !w.paused));
                if let Some((id, name, new_paused)) = pause {
                    self.db()?.set_paused(id, new_paused)?;
                    self.reload_watches()?;
                    self.request_reload().await;
                    self.flash.push(if new_paused {
                        format!("Pausado: {name}")
                    } else {
                        format!("Retomado: {name}")
                    });
                }
            }
            KeyCode::Char('r') => {
                let force = self
                    .watches
                    .get(self.selected)
                    .map(|w| (w.id, w.display_name().to_string()));
                if let Some((id, name)) = force {
                    self.request_force_check(id).await;
                    self.flash.push(format!("Checando agora: {name}"));
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
            KeyCode::Char('m') => {
                if let Some(w) = self.watches.get(self.selected) {
                    if w.unread_change {
                        self.db()?.mark_changes_seen_for_watch(w.id)?;
                    } else {
                        self.db()?.mark_all_changes_seen()?;
                    }
                } else {
                    self.db()?.mark_all_changes_seen()?;
                }
                self.reload_watches()?;
                self.reload_alerts()?;
                self.flash
                    .push("Marcado como lido.".into());
            }
            KeyCode::Char('t') => {
                self.settings.theme = self.settings.theme.toggle();
                theme::set_mode(self.settings.theme);
                self.db()?.save_settings(&self.settings)?;
            }
            KeyCode::Char('c') => {
                self.settings.density = self.settings.density.toggle();
                self.db()?.save_settings(&self.settings)?;
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
        if self.form.preview.is_some() {
            match code {
                KeyCode::Esc | KeyCode::Char('v') => {
                    self.form.preview = None;
                    self.form.preview_scroll = 0;
                }
                KeyCode::PageDown => self.form.preview_scroll = self.form.preview_scroll.saturating_add(5),
                KeyCode::PageUp => self.form.preview_scroll = self.form.preview_scroll.saturating_sub(5),
                _ => {}
            }
            return Ok(());
        }

        match code {
            KeyCode::Esc => self.screen = Screen::Dashboard,
            KeyCode::Tab | KeyCode::Down => self.form.next_field(),
            KeyCode::BackTab | KeyCode::Up => self.form.prev_field(),
            KeyCode::F(5) => self.run_preview().await?,
            KeyCode::Char('v')
                if matches!(
                    self.form.focus,
                    FormField::Url | FormField::Label | FormField::Interval
                ) =>
            {
                self.run_preview().await?;
            }
            KeyCode::Enter => self.submit_form().await?,
            KeyCode::Backspace => {
                self.form.focused_mut().pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                self.form.focused_mut().push(c);
            }
            _ => {}
        }
        Ok(())
    }

    async fn run_preview(&mut self) -> Result<()> {
        self.form.error = None;
        let url = self.form.url.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            self.form.error = Some("URL inválida para prévia".into());
            return Ok(());
        }
        let client = reqwest::Client::builder()
            .user_agent("Curio/0.2 (+MACAPE)")
            .timeout(Duration::from_secs(15))
            .build()?;
        let resp = client.get(url).send().await;
        match resp {
            Ok(r) if r.status().is_success() => match r.text().await {
                Ok(html) => match normalize_and_hash(&html, &self.form.selector, &self.form.ignore_regex)
                {
                    Ok((text, hash)) => {
                        let preview = if text.is_empty() {
                            "(vazio após normalização/filtros)".into()
                        } else {
                            format!("hash: {hash}\n\n{text}")
                        };
                        self.form.preview = Some(preview);
                        self.form.preview_scroll = 0;
                    }
                    Err(e) => self.form.error = Some(e.to_string()),
                },
                Err(e) => self.form.error = Some(format!("ler corpo: {e}")),
            },
            Ok(r) => self.form.error = Some(format!("HTTP {}", r.status())),
            Err(e) => self.form.error = Some(format!("rede: {e}")),
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
        // Validate selector / ignore by dry-run on empty html only for regex compile
        if let Err(e) = normalize_and_hash(
            "<html><body>teste</body></html>",
            "",
            &self.form.ignore_regex,
        ) {
            self.form.error = Some(e.to_string());
            self.form.focus = FormField::IgnoreRegex;
            return Ok(());
        }
        if !self.form.selector.trim().is_empty() {
            if let Err(e) = normalize_and_hash(
                "<html><body><div id=\"x\">ok</div></body></html>",
                self.form.selector.trim(),
                "",
            ) {
                // NoMatch is ok for validation of CSS syntax on sample; InvalidCss is not
                if !matches!(e, crate::extract::ExtractError::NoMatch) {
                    // XPath unsupported or invalid css
                    if matches!(e, crate::extract::ExtractError::InvalidCss(_))
                        || matches!(e, crate::extract::ExtractError::XPathUnsupported)
                    {
                        self.form.error = Some(e.to_string());
                        self.form.focus = FormField::Selector;
                        return Ok(());
                    }
                }
            }
        }

        let label = self.form.label.trim().to_string();
        let selector = self.form.selector.trim().to_string();
        let ignore_regex = self.form.ignore_regex.clone();
        let db = self.db()?;
        if let Some(id) = self.form.editing_id {
            db.update_watch(id, &url, &label, interval, &selector, &ignore_regex)?;
        } else {
            db.insert_watch(&url, &label, interval, &selector, &ignore_regex)?;
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
Curió v0.2 — atalhos

  a          adicionar monitor
  e          editar selecionado
  d          excluir selecionado
  p          pausar / retomar
  r          checar agora
  o          abrir URL no navegador
  h          histórico / diferenças
  l          painel de alertas
  m          marcar lido (selecionado ou todos)
  t          alternar tema claro/escuro
  c          densidade normal/compacta
  v / F5     prévia do texto (no formulário)
  ?          esta ajuda
  q          sair

Seletor: CSS (#id, .classe) ou XPath comum (//*[@id='x']).
Ignorar: uma regex por linha; linhas casadas saem da comparação.
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
            .border_style(ratatui::style::Style::default().fg(theme::accent_fg())),
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
