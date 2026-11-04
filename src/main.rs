//! Curió — TUI para monitorar mudanças em URLs.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

mod app;
mod db;
mod diff;
mod extract;
mod interval;
mod model;
mod monitor;
mod notify;
mod ui;

use crate::app::App;
use crate::db::{default_db_path, Database};
use crate::monitor::{run_monitor, ScheduleMap};
use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::collections::HashMap;
use std::io::stdout;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[tokio::main]
async fn main() -> Result<()> {
    let db_path = default_db_path()?;
    // Ensure DB exists / migrated
    let _ = Database::open_path(&db_path)?;

    let (tx, rx) = mpsc::unbounded_channel();
    let schedule: ScheduleMap = Arc::new(Mutex::new(HashMap::new()));
    let reload = Arc::new(Mutex::new(false));

    let mon_path = db_path.clone();
    let mon_schedule = schedule.clone();
    let mon_reload = reload.clone();
    tokio::spawn(async move {
        run_monitor(mon_path, tx, mon_schedule, mon_reload).await;
    });

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(db_path, schedule, reload, rx)?;
    let result = app.run(&mut terminal).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("erro: {e:#}");
        return Err(e);
    }
    Ok(())
}
