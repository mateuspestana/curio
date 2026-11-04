//! Async URL monitor with retry policy, pause, and force-check.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::db::Database;
use crate::diff::unified_diff;
use crate::extract::normalize_and_hash;
use crate::model::{
    AppEvent, WatchStatus, FETCH_TIMEOUT_SECS, MAX_FETCH_ATTEMPTS, RETRY_INTERVAL_SECS,
};
use crate::notify::{notify_change, notify_downtime};
use anyhow::Result;
use chrono::{Duration, Utc};
use reqwest::Client;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, Instant};

/// Shared next-check schedule visible to the TUI (watch_id -> Instant).
pub type ScheduleMap = Arc<Mutex<HashMap<i64, Instant>>>;
/// Watch IDs requested for immediate check.
pub type ForceCheckSet = Arc<Mutex<HashSet<i64>>>;

pub async fn run_monitor(
    db_path: PathBuf,
    tx: mpsc::UnboundedSender<AppEvent>,
    schedule: ScheduleMap,
    reload: Arc<Mutex<bool>>,
    force: ForceCheckSet,
) {
    let client = Client::builder()
        .user_agent("Curio/0.2 (+MACAPE; https://github.com/mateuspestana/curio)")
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
        .expect("http client");

    let mut due: HashMap<i64, Instant> = HashMap::new();

    loop {
        {
            let mut flag = reload.lock().await;
            if *flag {
                *flag = false;
                due.clear();
            }
        }

        let forced: HashSet<i64> = {
            let mut set = force.lock().await;
            let copy = set.clone();
            set.clear();
            copy
        };

        let watches = match open_db(&db_path).and_then(|db| db.list_watches()) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("erro no banco do monitor: {e}");
                sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };

        let now = Instant::now();
        let watch_ids: Vec<i64> = watches.iter().map(|w| w.id).collect();
        due.retain(|id, _| watch_ids.contains(id));

        for w in &watches {
            if w.paused {
                due.remove(&w.id);
                continue;
            }
            due.entry(w.id).or_insert(now);
        }

        {
            let mut map = schedule.lock().await;
            *map = due.clone();
        }

        let mut next_wake = now + std::time::Duration::from_secs(1);
        let mut ran_any = false;

        for w in watches {
            if w.paused && !forced.contains(&w.id) {
                continue;
            }

            let force_this = forced.contains(&w.id);
            let when = *due.get(&w.id).unwrap_or(&now);
            if !force_this && when > now {
                if when < next_wake {
                    next_wake = when;
                }
                continue;
            }

            ran_any = true;
            let result = check_watch(&client, &db_path, w.id, force_this).await;
            match result {
                CheckOutcome::Ok { watch, changed } => {
                    due.insert(
                        watch.id,
                        Instant::now() + std::time::Duration::from_secs(watch.interval_secs),
                    );
                    let _ = tx.send(AppEvent::WatchUpdated(watch.clone()));
                    if changed {
                        let label = display_label(&watch.label, &watch.url);
                        let _ = tx.send(AppEvent::ChangeDetected {
                            watch_id: watch.id,
                            label: label.clone(),
                        });
                        notify_change(&watch.label, &watch.url);
                    }
                }
                CheckOutcome::Retry { watch } => {
                    due.insert(
                        watch.id,
                        Instant::now() + std::time::Duration::from_secs(RETRY_INTERVAL_SECS),
                    );
                    let _ = tx.send(AppEvent::WatchUpdated(watch));
                }
                CheckOutcome::Downtime { watch, error } => {
                    due.insert(
                        watch.id,
                        Instant::now() + std::time::Duration::from_secs(watch.interval_secs),
                    );
                    let label = display_label(&watch.label, &watch.url);
                    notify_downtime(&watch.label, &watch.url, &error);
                    let _ = tx.send(AppEvent::Downtime {
                        watch_id: watch.id,
                        label,
                        error,
                    });
                    let _ = tx.send(AppEvent::WatchUpdated(watch));
                }
                CheckOutcome::SkippedPaused => {}
                CheckOutcome::Fatal(e) => {
                    eprintln!("falha fatal na verificação: {e}");
                    due.insert(w.id, Instant::now() + std::time::Duration::from_secs(60));
                }
            }
        }

        if !ran_any {
            let sleep_for = next_wake.saturating_duration_since(Instant::now());
            let sleep_for = sleep_for.min(std::time::Duration::from_millis(400));
            sleep(sleep_for).await;
            let _ = tx.send(AppEvent::Tick);
        } else {
            let _ = tx.send(AppEvent::Tick);
        }
    }
}

enum CheckOutcome {
    Ok { watch: crate::model::Watch, changed: bool },
    Retry { watch: crate::model::Watch },
    Downtime { watch: crate::model::Watch, error: String },
    SkippedPaused,
    Fatal(anyhow::Error),
}

async fn check_watch(
    client: &Client,
    db_path: &PathBuf,
    watch_id: i64,
    force: bool,
) -> CheckOutcome {
    let db = match open_db(db_path) {
        Ok(d) => d,
        Err(e) => return CheckOutcome::Fatal(e),
    };
    let watch = match db.get_watch(watch_id) {
        Ok(Some(w)) => w,
        Ok(None) => return CheckOutcome::Fatal(anyhow::anyhow!("watch removido")),
        Err(e) => return CheckOutcome::Fatal(e),
    };

    if watch.paused && !force {
        return CheckOutcome::SkippedPaused;
    }

    let fetch = client.get(&watch.url).send().await;
    let body = match fetch {
        Ok(resp) => {
            let status = resp.status();
            if !status.is_success() {
                Err(format!("HTTP {status}"))
            } else {
                resp.text()
                    .await
                    .map_err(|e| format!("ler corpo: {e}"))
            }
        }
        Err(e) => Err(format!("rede: {e}")),
    };

    match body {
        Ok(html) => {
            let (text, hash) = match normalize_and_hash(&html, &watch.selector, &watch.ignore_regex)
            {
                Ok(v) => v,
                Err(e) => {
                    let msg = e.to_string();
                    if let Err(err) =
                        db.update_watch_check(watch_id, WatchStatus::Error, Some(&msg), 0, None)
                    {
                        return CheckOutcome::Fatal(err);
                    }
                    let updated = match db.get_watch(watch_id) {
                        Ok(Some(w)) => w,
                        Ok(None) => return CheckOutcome::Fatal(anyhow::anyhow!("watch removido")),
                        Err(err) => return CheckOutcome::Fatal(err),
                    };
                    return CheckOutcome::Downtime {
                        watch: updated,
                        error: msg,
                    };
                }
            };

            let prev = match db.latest_snapshot(watch_id) {
                Ok(s) => s,
                Err(e) => return CheckOutcome::Fatal(e),
            };

            let mut changed = false;
            match prev {
                None => {
                    if let Err(e) = db.insert_snapshot(watch_id, &hash, &text) {
                        return CheckOutcome::Fatal(e);
                    }
                    if let Err(e) =
                        db.update_watch_check(watch_id, WatchStatus::Ok, None, 0, Some(false))
                    {
                        return CheckOutcome::Fatal(e);
                    }
                }
                Some(prev) if prev.content_hash == hash => {
                    if let Err(e) = db.update_watch_check(watch_id, WatchStatus::Ok, None, 0, None)
                    {
                        return CheckOutcome::Fatal(e);
                    }
                }
                Some(prev) => {
                    let diff = unified_diff(&prev.normalized_text, &text);
                    if let Err(e) = db.insert_snapshot(watch_id, &hash, &text) {
                        return CheckOutcome::Fatal(e);
                    }
                    if let Err(e) = db.insert_change(watch_id, &diff) {
                        return CheckOutcome::Fatal(e);
                    }
                    if let Err(e) =
                        db.update_watch_check(watch_id, WatchStatus::Changed, None, 0, Some(true))
                    {
                        return CheckOutcome::Fatal(e);
                    }
                    changed = true;
                }
            }

            if watch.paused {
                let _ = db.set_paused(watch_id, true);
            }

            match db.get_watch(watch_id) {
                Ok(Some(w)) => CheckOutcome::Ok { watch: w, changed },
                Ok(None) => CheckOutcome::Fatal(anyhow::anyhow!("watch removido")),
                Err(e) => CheckOutcome::Fatal(e),
            }
        }
        Err(error) => {
            let failures = watch.consecutive_failures.saturating_add(1);
            if let Err(e) =
                db.update_watch_check(watch_id, WatchStatus::Error, Some(&error), failures, None)
            {
                return CheckOutcome::Fatal(e);
            }
            let updated = match db.get_watch(watch_id) {
                Ok(Some(w)) => w,
                Ok(None) => return CheckOutcome::Fatal(anyhow::anyhow!("watch removido")),
                Err(e) => return CheckOutcome::Fatal(e),
            };

            if failures < MAX_FETCH_ATTEMPTS {
                CheckOutcome::Retry { watch: updated }
            } else {
                let _ = db.update_watch_check(
                    watch_id,
                    WatchStatus::Error,
                    Some(&error),
                    0,
                    None,
                );
                let updated = db.get_watch(watch_id).ok().flatten().unwrap_or(updated);
                CheckOutcome::Downtime {
                    watch: updated,
                    error,
                }
            }
        }
    }
}

fn open_db(path: &PathBuf) -> Result<Database> {
    Database::open_path(path)
}

fn display_label(label: &str, url: &str) -> String {
    if label.is_empty() {
        url.to_string()
    } else {
        label.to_string()
    }
}

#[allow(dead_code)]
pub fn eta_from_instant(target: Instant) -> chrono::DateTime<Utc> {
    let dur = target.saturating_duration_since(Instant::now());
    Utc::now() + Duration::from_std(dur).unwrap_or_else(|_| Duration::seconds(0))
}
