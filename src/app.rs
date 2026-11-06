//! Application state and TUI event loop.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::condition::Condition;
use crate::db::Database;
use crate::fetch::{build_client, fetch_and_extract, find_browser, Target};
use crate::model::{AppEvent, AppSettings, Change, Mode, Watch};
use crate::monitor::{ForceCheckSet, ScheduleMap};
use crate::spec::WatchSpec;
use crate::transfer::{
    export_history, export_warnings, export_watches, write_file, ConfigFormat, HistoryFormat,
};
use crate::ui::dashboard::DashboardView;
use crate::ui::form::{FormField, FormState};
use crate::ui::history::DiffView;
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

const NO_BROWSER_MSG: &str =
    "modo js precisa de Chrome/Chromium/Brave/Edge (ou defina CURIO_CHROME com o caminho)";

/// Quanto tempo uma mensagem passageira fica no rodapé.
const TOAST_TTL: Duration = Duration::from_secs(6);
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
    /// Todos os monitores do banco.
    all_watches: Vec<Watch>,
    /// Monitores visíveis (após busca/tag). `selected` indexa ESTA lista, então
    /// toda ação (excluir, pausar, editar…) atua no item que está na tela.
    watches: Vec<Watch>,
    selected: usize,
    filter: String,
    filtering: bool,
    tag_filter: Option<String>,
    toast: Option<(String, std::time::Instant)>,
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
            all_watches: Vec::new(),
            watches: Vec::new(),
            selected: 0,
            filter: String::new(),
            filtering: false,
            tag_filter: None,
            toast: None,
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
        let keep = self.watches.get(self.selected).map(|w| w.id);
        self.all_watches = self.db.list_watches()?;
        self.unread_total = self.db.unread_count()?;
        self.apply_filters(keep);
        Ok(())
    }

    /// Recalcula a lista visível; mantém a seleção no mesmo monitor (por id).
    fn apply_filters(&mut self, keep: Option<i64>) {
        if let Some(tag) = &self.tag_filter {
            if !self
                .all_watches
                .iter()
                .any(|w| w.tag_list().any(|t| t == tag))
            {
                self.tag_filter = None;
            }
        }
        let needle = self.filter.trim().to_lowercase();
        let tag = self.tag_filter.clone();
        self.watches = self
            .all_watches
            .iter()
            .filter(|w| tag.as_ref().is_none_or(|t| w.tag_list().any(|x| x == t)))
            .filter(|w| {
                needle.is_empty()
                    || w.label.to_lowercase().contains(&needle)
                    || w.url.to_lowercase().contains(&needle)
                    || w.tags.contains(&needle)
            })
            .cloned()
            .collect();
        self.selected = keep
            .and_then(|id| self.watches.iter().position(|w| w.id == id))
            .unwrap_or(self.selected)
            .min(self.watches.len().saturating_sub(1));
    }

    /// Etiqueta do filtro ativo para o título da lista.
    fn filter_note(&self) -> String {
        let mut parts = Vec::new();
        if let Some(t) = &self.tag_filter {
            parts.push(format!("tag #{t}"));
        }
        if self.filtering || !self.filter.is_empty() {
            parts.push(format!(
                "busca \"{}\"{}",
                self.filter,
                if self.filtering { "█" } else { "" }
            ));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(
                "{} · {}/{}",
                parts.join(" + "),
                self.watches.len(),
                self.all_watches.len()
            )
        }
    }

    fn cycle_tag_filter(&mut self) {
        let mut tags: Vec<String> = self
            .all_watches
            .iter()
            .flat_map(|w| w.tag_list().map(String::from))
            .collect();
        tags.sort();
        tags.dedup();
        self.tag_filter = match &self.tag_filter {
            None => tags.first().cloned(),
            Some(cur) => tags
                .iter()
                .position(|t| t == cur)
                .and_then(|i| tags.get(i + 1))
                .cloned(),
        };
        let keep = self.watches.get(self.selected).map(|w| w.id);
        self.apply_filters(keep);
    }

    /// Mensagem no rodapé (some sozinha) e no painel de alertas.
    fn say(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.toast = Some((msg.clone(), std::time::Instant::now()));
        self.push_flash(msg);
    }

    fn active_toast(&self) -> Option<&str> {
        self.toast
            .as_ref()
            .filter(|(_, t)| t.elapsed() < TOAST_TTL)
            .map(|(m, _)| m.as_str())
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

    pub async fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
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
                    if self.toast.as_ref().is_some_and(|(_, t)| t.elapsed() >= TOAST_TTL) {
                        self.toast = None;
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
                self.say(format!("Mudança detectada: {label}"));
                reload.alerts = true;
            }
            AppEvent::Downtime { label, error, .. } => {
                self.say(format!("Indisponível: {label} — {error}"));
            }
            AppEvent::Recovered { label, .. } => {
                self.say(format!("Voltou ao ar: {label}"));
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
                    DashboardView {
                        watches: &self.watches,
                        selected: self.selected,
                        schedule,
                        density: self.settings.density,
                        unread_total: self.unread_total,
                        filter_note: &self.filter_note(),
                    },
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
                    DiffView {
                        context: self.settings.diff_context,
                        side_by_side: self.settings.diff_side_by_side,
                    },
                );
            }
            Screen::Alerts => alerts::draw(frame, body, &self.alerts, &self.flash),
            Screen::Help => draw_help(frame, body),
        }

        let width = footer.width.saturating_sub(4) as usize;
        let hints = match self.screen {
            Screen::Dashboard if self.filtering => {
                format!("/{}█ · ↑↓ navega · Enter aceita · Esc limpa", self.filter)
            }
            Screen::Dashboard => widgets::fit_hints(
                &[
                    "a add",
                    "e edit",
                    "d del",
                    "p pausa",
                    "r checar",
                    "h hist",
                    "l alertas",
                    "m lidos",
                    "/ busca",
                    "g tag",
                    "o abrir",
                    "x export",
                    "t tema",
                    "c dens",
                ],
                "? ajuda · q sair",
                width,
            ),
            Screen::Form => widgets::fit_hints(
                &["Tab campo", "F5 prévia", "Enter salvar"],
                "Esc cancelar",
                width,
            ),
            Screen::History => widgets::fit_hints(
                &[
                    "↑↓",
                    "c contexto",
                    "s lado a lado",
                    "x exportar",
                    "m visto",
                    "o abrir",
                ],
                "Esc voltar",
                width,
            ),
            Screen::Alerts => "m marcar todos vistos · Esc voltar".into(),
            Screen::Help | Screen::ConfirmDelete => "Esc voltar".into(),
        };
        widgets::draw_footer(frame, footer, &hints, self.active_toast());
    }

    async fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            self.should_quit = true;
            return Ok(());
        }

        match self.screen {
            Screen::Dashboard => self.key_dashboard(code).await?,
            Screen::Form => self.key_form(code, mods).await?,
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

    /// Edição da busca (`/`): atualiza a lista a cada tecla.
    fn key_filtering(&mut self, code: KeyCode) {
        let keep = self.watches.get(self.selected).map(|w| w.id);
        match code {
            KeyCode::Enter => self.filtering = false,
            KeyCode::Esc => {
                self.filter.clear();
                self.filtering = false;
            }
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(c) if !c.is_control() => self.filter.push(c),
            KeyCode::Down if !self.watches.is_empty() => {
                self.selected = (self.selected + 1) % self.watches.len();
                return;
            }
            KeyCode::Up if !self.watches.is_empty() => {
                self.selected = self
                    .selected
                    .checked_sub(1)
                    .unwrap_or(self.watches.len() - 1);
                return;
            }
            _ => return,
        }
        self.apply_filters(keep);
    }

    async fn key_dashboard(&mut self, code: KeyCode) -> Result<()> {
        if self.filtering {
            self.key_filtering(code);
            return Ok(());
        }
        match code {
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('g') => {
                self.cycle_tag_filter();
                let msg = match &self.tag_filter {
                    Some(t) => format!("Filtrando pela tag #{t}"),
                    None => "Mostrando todas as tags".into(),
                };
                self.say(msg);
            }
            KeyCode::Char('x') => self.export_monitors(),
            KeyCode::Esc if !self.filter.is_empty() || self.tag_filter.is_some() => {
                self.filter.clear();
                self.tag_filter = None;
                let keep = self.watches.get(self.selected).map(|w| w.id);
                self.apply_filters(keep);
            }
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.screen = Screen::Help,
            KeyCode::Char('a') => {
                self.form = FormState::new_add();
                self.screen = Screen::Form;
            }
            KeyCode::Char('e') => {
                if let Some(w) = self.watches.get(self.selected) {
                    self.form = FormState::from_spec(Some(w.id), &WatchSpec::from_watch(w));
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
                    self.say(if new_paused {
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
                    self.say(format!("Checando agora: {name}"));
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
                self.say("Marcado como lido.");
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
            KeyCode::Up | KeyCode::Char('k') if !self.watches.is_empty() => {
                self.selected = self
                    .selected
                    .checked_sub(1)
                    .unwrap_or(self.watches.len() - 1);
            }
            _ => {}
        }
        Ok(())
    }

    async fn key_form(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        if self.form.preview.is_some() {
            match code {
                KeyCode::Esc | KeyCode::Char('v') => {
                    self.form.preview = None;
                    self.form.preview_scroll = 0;
                }
                KeyCode::PageDown => {
                    self.form.preview_scroll = self.form.preview_scroll.saturating_add(5)
                }
                KeyCode::PageUp => {
                    self.form.preview_scroll = self.form.preview_scroll.saturating_sub(5)
                }
                _ => {}
            }
            return Ok(());
        }

        let ctrl = mods.contains(KeyModifiers::CONTROL);
        match code {
            KeyCode::Esc => self.screen = Screen::Dashboard,
            KeyCode::Tab | KeyCode::Down => self.form.next_field(),
            KeyCode::BackTab | KeyCode::Up => self.form.prev_field(),
            KeyCode::F(5) => self.run_preview().await?,
            KeyCode::Enter => self.submit_form().await?,
            KeyCode::Char('n') if ctrl => {
                if self.form.focus.is_multiline() {
                    if let Some(buf) = self.form.focused_mut() {
                        buf.push('\n');
                    }
                }
            }
            KeyCode::Left if self.form.focus == FormField::Mode => {
                self.form.mode = self.form.mode.prev()
            }
            KeyCode::Right | KeyCode::Char(' ') if self.form.focus == FormField::Mode => {
                self.form.mode = self.form.mode.next()
            }
            KeyCode::Backspace => {
                if let Some(buf) = self.form.focused_mut() {
                    buf.pop();
                }
            }
            KeyCode::Char(c) if !ctrl && !c.is_control() => {
                if let Some(buf) = self.form.focused_mut() {
                    buf.push(c);
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn run_preview(&mut self) -> Result<()> {
        self.form.error = None;
        let url = self.form.url.trim().to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            self.form.error = Some("URL inválida para prévia".into());
            self.form.focus = FormField::Url;
            return Ok(());
        }
        if self.form.mode == Mode::Js && find_browser().is_none() {
            self.form.error = Some(NO_BROWSER_MSG.into());
            self.form.focus = FormField::Mode;
            return Ok(());
        }
        let target = Target {
            url,
            mode: self.form.mode,
            selector: self.form.selector.trim().to_string(),
            ignore_regex: self.form.ignore_regex.clone(),
            headers: self.form.headers.clone(),
        };
        // Mesma pipeline do monitor: modo, cabeçalhos e extração idênticos.
        match fetch_and_extract(&build_client(), &target).await {
            Ok(ex) => {
                let mut preview = String::new();
                match Condition::parse(&self.form.condition) {
                    Ok(c) if c.is_empty() => {}
                    Ok(c) => preview.push_str(if c.matches(&ex.text) {
                        "condição: ATENDE agora (alerta quando passar de \"não atende\" para \"atende\")\n"
                    } else {
                        "condição: NÃO atende agora\n"
                    }),
                    Err(e) => preview.push_str(&format!("condição inválida: {e}\n")),
                }
                if ex.text.is_empty() {
                    preview.push_str("(vazio após normalização/filtros)");
                } else {
                    preview.push_str(&format!("hash: {}\n\n{}", ex.hash, ex.text));
                }
                self.form.preview = Some(preview);
                self.form.preview_scroll = 0;
            }
            Err(e) => self.form.error = Some(e.message),
        }
        Ok(())
    }

    async fn submit_form(&mut self) -> Result<()> {
        self.form.error = None;
        let spec = match self.form.to_spec() {
            Ok(spec) => spec,
            Err((field, msg)) => {
                self.form.error = Some(msg);
                self.form.focus = field;
                return Ok(());
            }
        };
        if let Err(e) = spec.validate() {
            self.form.error = Some(e.message);
            self.form.focus = e.field.into();
            return Ok(());
        }
        if spec.mode == Mode::Js && find_browser().is_none() {
            self.form.error = Some(NO_BROWSER_MSG.into());
            self.form.focus = FormField::Mode;
            return Ok(());
        }

        if let Some(id) = self.form.editing_id {
            self.db.update_watch(id, &spec)?;
        } else {
            self.db.insert_watch(&spec)?;
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
                    let id = w.id;
                    self.db.mark_changes_seen_for_watch(id)?;
                    self.changes = self.db.list_changes(id, 100)?;
                    self.reload_watches()?;
                }
            }
            KeyCode::Char('c') => {
                self.settings.diff_context = !self.settings.diff_context;
                self.db.save_settings(&self.settings)?;
                self.diff_scroll = 0;
            }
            KeyCode::Char('s') => {
                self.settings.diff_side_by_side = !self.settings.diff_side_by_side;
                self.db.save_settings(&self.settings)?;
                self.diff_scroll = 0;
            }
            KeyCode::Char('x') => self.export_selected_history(),
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.changes.is_empty() {
                    self.change_selected = (self.change_selected + 1) % self.changes.len();
                    self.diff_scroll = 0;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.changes.is_empty() {
                    self.change_selected = self
                        .change_selected
                        .checked_sub(1)
                        .unwrap_or(self.changes.len() - 1);
                    self.diff_scroll = 0;
                }
            }
            KeyCode::PageDown => self.diff_scroll = self.diff_scroll.saturating_add(5),
            KeyCode::PageUp => self.diff_scroll = self.diff_scroll.saturating_sub(5),
            _ => {}
        }
        Ok(())
    }

    /// `~/.curio/exports` (ou ao lado do banco indicado em `CURIO_DB`).
    fn export_dir() -> Result<PathBuf> {
        let dir = crate::db::data_dir()?.join("exports");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn export_monitors(&mut self) {
        let result = (|| -> Result<(PathBuf, usize)> {
            let watches = self.db.list_watches()?;
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let path = Self::export_dir()?.join(format!("monitores-{stamp}.toml"));
            // Pode conter cabeçalhos: arquivo só do usuário.
            write_file(&path, &export_watches(&watches, ConfigFormat::Toml)?, true)?;
            Ok((path, export_warnings(&watches).len()))
        })();
        match result {
            Ok((path, 0)) => self.say(format!("Monitores exportados: {}", path.display())),
            Ok((path, n)) => self.say(format!(
                "Exportado: {} (aviso: {n} cabeçalho(s) com valor literal; prefira ${{VAR}})",
                path.display()
            )),
            Err(e) => self.say(format!("Falha ao exportar: {e}")),
        }
    }

    fn export_selected_history(&mut self) {
        let Some(id) = self.watches.get(self.selected).map(|w| w.id) else {
            return;
        };
        let result = (|| -> Result<PathBuf> {
            let rows = self.db.list_changes_for_export(Some(id))?;
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let dir = Self::export_dir()?;
            for format in [HistoryFormat::Csv, HistoryFormat::Markdown] {
                let path = dir.join(format!("historico-{id}-{stamp}.{}", format.extension()));
                write_file(&path, &export_history(&rows, format), false)?;
            }
            Ok(dir)
        })();
        match result {
            Ok(dir) => self.say(format!(
                "Histórico exportado (CSV e MD) em {}",
                dir.display()
            )),
            Err(e) => self.say(format!("Falha ao exportar: {e}")),
        }
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
    let text = format!(
        "\
Curió v{} — atalhos

  a          adicionar monitor
  e          editar selecionado
  d          excluir selecionado
  p          pausar / retomar
  r          checar agora
  o          abrir URL no navegador
  h          histórico / diferenças
  /          buscar (rótulo, URL ou tag) · Enter aceita · Esc limpa
  g          filtrar por tag (repita para passar à próxima)
  x          exportar monitores (TOML) · no histórico: exporta CSV e Markdown
  l          painel de alertas
  m          marcar lido (selecionado ou todos)
  t          alternar tema claro/escuro
  c          densidade normal/compacta
  ?          esta ajuda
  q          sair

Formulário: Tab/Shift+Tab muda de campo · F5 prévia · Ctrl+N nova linha
  Modo       html (padrão) · json (API) · js (renderiza JavaScript com Chrome/Chromium)
  Seletor    CSS/XPath comum — ou, no modo json, caminho como items[*].price
  Ignorar    uma regex por linha; linhas casadas saem da comparação
  Alertar só se…  contém: x · não contém: x · regex: x · número < 100
             (alerta quando passa de falso para verdadeiro; o histórico guarda tudo)
  Cabeçalhos Nome: valor · use ${{VAR}} para segredos (lidos do ambiente)
  Tags       separadas por vírgula

Histórico: c contexto · s lado a lado · x exportar
CLI: curio export · import · history · config · prune (curio --help)
",
        env!("CARGO_PKG_VERSION")
    );
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
