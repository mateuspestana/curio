//! Async URL monitor with backoff, pause, force-check, conditions and retention.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::condition::Condition;
use crate::db::Database;
use crate::diff::{diff_with_context, CONTEXT_LINES};
use crate::fetch::{build_client, fetch_and_extract, Target};
use crate::model::{
    AppEvent, Watch, WatchStatus, MAX_BACKOFF_SECS, MAX_FETCH_ATTEMPTS, RETRY_BASE_SECS,
};
use crate::notify::{notify_change, notify_downtime, notify_recovery};
use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use reqwest::Client;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, Notify, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{sleep, Instant};

/// Shared next-check schedule visible to the TUI (watch_id -> Instant).
pub type ScheduleMap = Arc<Mutex<HashMap<i64, Instant>>>;
/// Watch IDs requested for immediate check.
pub type ForceCheckSet = Arc<Mutex<HashSet<i64>>>;

/// Máximo de URLs buscadas ao mesmo tempo.
const MAX_CONCURRENT_CHECKS: usize = 4;
/// Intervalo entre limpezas de retenção (e VACUUM, se algo foi apagado).
const PRUNE_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// Espera após a `failures`-ésima falha seguida: 60s, 120s, 240s… até 1h, nunca
/// menos que o `Retry-After` pedido pelo servidor.
pub fn failure_delay_secs(failures: u32, retry_after: Option<u64>) -> u64 {
    let shift = failures.saturating_sub(1).min(16);
    let backoff = RETRY_BASE_SECS
        .saturating_mul(1u64 << shift)
        .min(MAX_BACKOFF_SECS);
    backoff.max(retry_after.unwrap_or(0))
}

pub async fn run_monitor(
    db_path: PathBuf,
    tx: mpsc::UnboundedSender<AppEvent>,
    schedule: ScheduleMap,
    reload: Arc<Mutex<bool>>,
    force: ForceCheckSet,
    wake: Arc<Notify>,
) {
    let client = build_client();

    // Uma única conexão (migrada) para listar monitores; cada checagem abre a sua.
    let db = loop {
        match open_db(&db_path) {
            Ok(db) => break db,
            Err(e) => {
                eprintln!("erro no banco do monitor: {e}");
                sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    };
    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_CHECKS));
    let mut due: HashMap<i64, Instant> = HashMap::new();
    let mut next_prune = Instant::now();

    loop {
        if Instant::now() >= next_prune {
            let path = db_path.clone();
            let _ = tokio::task::spawn_blocking(move || run_prune(&path)).await;
            next_prune = Instant::now() + PRUNE_EVERY;
        }

        {
            let mut flag = reload.lock().await;
            if *flag {
                *flag = false;
                due.clear();
            }
        }

        let forced: HashSet<i64> = std::mem::take(&mut *force.lock().await);

        let watches = match db.list_watches() {
            Ok(w) => w,
            Err(e) => {
                eprintln!("erro no banco do monitor: {e}");
                sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };

        let now = Instant::now();
        let ids: HashSet<i64> = watches.iter().map(|w| w.id).collect();
        due.retain(|id, _| ids.contains(id));
        for w in &watches {
            if w.paused {
                due.remove(&w.id);
            } else {
                due.entry(w.id).or_insert(now);
            }
        }

        // Monitores vencidos (ou forçados) nesta rodada.
        let to_run: Vec<i64> = watches
            .iter()
            .filter(|w| {
                forced.contains(&w.id) || (!w.paused && due.get(&w.id).is_none_or(|t| *t <= now))
            })
            .map(|w| w.id)
            .collect();

        if !to_run.is_empty() {
            let mut tasks = JoinSet::new();
            for id in to_run {
                let client = client.clone();
                let path = db_path.clone();
                let permits = permits.clone();
                let force_this = forced.contains(&id);
                tasks.spawn(async move {
                    let _permit = permits.acquire_owned().await;
                    (id, check_watch(&client, &path, id, force_this).await)
                });
            }
            while let Some(joined) = tasks.join_next().await {
                let Ok((id, result)) = joined else { continue };
                handle_outcome(id, result, &mut due, &tx);
            }
        }

        *schedule.lock().await = due.clone();

        // Dorme até o próximo vencimento; reload/forçar/pausar acordam na hora.
        let next = due
            .values()
            .min()
            .copied()
            .map_or(next_prune, |t| t.min(next_prune));
        tokio::select! {
            _ = tokio::time::sleep_until(next) => {}
            _ = wake.notified() => {}
        }
    }
}

/// Retenção do histórico; só faz VACUUM se algo foi apagado.
fn run_prune(path: &Path) -> Result<()> {
    let db = Database::connect(path)?;
    if db.prune()? > 0 {
        db.vacuum()?;
    }
    Ok(())
}

fn handle_outcome(
    id: i64,
    result: CheckOutcome,
    due: &mut HashMap<i64, Instant>,
    tx: &mpsc::UnboundedSender<AppEvent>,
) {
    match result {
        CheckOutcome::Ok {
            watch,
            alert,
            recovered,
            ..
        } => {
            due.insert(
                watch.id,
                Instant::now() + std::time::Duration::from_secs(watch.interval_secs),
            );
            let label = display_label(&watch.label, &watch.url);
            let _ = tx.send(AppEvent::WatchUpdated(watch.id));
            if recovered {
                notify_recovery(&watch.label, &watch.url);
                let _ = tx.send(AppEvent::Recovered {
                    watch_id: watch.id,
                    label: label.clone(),
                });
            }
            if alert {
                let _ = tx.send(AppEvent::ChangeDetected {
                    watch_id: watch.id,
                    label,
                });
                notify_change(&watch.label, &watch.url);
            }
        }
        CheckOutcome::Failed {
            watch,
            error,
            delay_secs,
            notify_down,
        } => {
            due.insert(
                watch.id,
                Instant::now() + std::time::Duration::from_secs(delay_secs),
            );
            if notify_down {
                let label = display_label(&watch.label, &watch.url);
                notify_downtime(&watch.label, &watch.url, &error);
                let _ = tx.send(AppEvent::Downtime {
                    watch_id: watch.id,
                    label,
                    error,
                });
            }
            let _ = tx.send(AppEvent::WatchUpdated(watch.id));
        }
        CheckOutcome::SkippedPaused => {
            due.remove(&id);
        }
        CheckOutcome::Fatal(e) => {
            eprintln!("falha fatal na verificação: {e}");
            due.insert(id, Instant::now() + std::time::Duration::from_secs(60));
        }
    }
}

enum CheckOutcome {
    Ok {
        watch: Watch,
        /// O texto mudou desde o último snapshot.
        #[allow(dead_code)]
        changed: bool,
        /// A mudança atende à condição: notificar e marcar como não lida.
        alert: bool,
        /// Voltou a responder depois de um downtime já avisado.
        recovered: bool,
    },
    Failed {
        watch: Watch,
        error: String,
        delay_secs: u64,
        /// Primeira vez que atinge o limite de falhas (avisa uma só vez).
        notify_down: bool,
    },
    SkippedPaused,
    Fatal(anyhow::Error),
}

async fn check_watch(client: &Client, db_path: &Path, watch_id: i64, force: bool) -> CheckOutcome {
    let db = match Database::connect(db_path) {
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

    let outcome = match fetch_and_extract(client, &Target::from_watch(&watch)).await {
        Ok(ex) => record_success(&db, &watch, &ex.text, &ex.hash),
        Err(err) => record_failure(&db, &watch, &err.message, err.retry_after_secs),
    };
    outcome.unwrap_or_else(CheckOutcome::Fatal)
}

fn record_success(db: &Database, watch: &Watch, text: &str, hash: &str) -> Result<CheckOutcome> {
    let id = watch.id;
    let mut changed = false;
    let mut alert = false;

    match db.latest_snapshot(id)? {
        None => {
            db.insert_snapshot(id, hash, text)?;
            db.update_watch_check(id, WatchStatus::Ok, None, 0, Some(false))?;
        }
        Some(prev) if prev.content_hash == hash => {
            db.update_watch_check(id, WatchStatus::Ok, None, 0, None)?;
        }
        Some(prev) => {
            let diff = diff_with_context(&prev.normalized_text, text, CONTEXT_LINES);
            // Condição inválida no banco não pode silenciar o monitor: alerta sempre.
            let condition = Condition::parse(&watch.condition).unwrap_or_default();
            alert = condition.should_alert(&prev.normalized_text, text);
            db.insert_snapshot(id, hash, text)?;
            // O diff sempre vai para o histórico; a condição só decide o alerta.
            db.insert_change(id, &diff, !alert)?;
            let status = if alert {
                WatchStatus::Changed
            } else {
                WatchStatus::Ok
            };
            db.update_watch_check(id, status, None, 0, alert.then_some(true))?;
            changed = true;
        }
    }

    let recovered = watch.down_since.is_some();
    if recovered {
        db.set_down_since(id, None)?;
    }
    if watch.paused {
        db.set_paused(id, true)?;
    }

    let watch = db.get_watch(id)?.context("watch removido")?;
    Ok(CheckOutcome::Ok {
        watch,
        changed,
        alert,
        recovered,
    })
}

fn record_failure(
    db: &Database,
    watch: &Watch,
    error: &str,
    retry_after: Option<u64>,
) -> Result<CheckOutcome> {
    let id = watch.id;
    // A contagem não zera ao avisar: o backoff continua crescendo enquanto estiver fora.
    let failures = watch.consecutive_failures.saturating_add(1);
    db.update_watch_check(id, WatchStatus::Error, Some(error), failures, None)?;

    let notify_down = failures >= MAX_FETCH_ATTEMPTS && watch.down_since.is_none();
    if notify_down {
        db.set_down_since(id, Some(Utc::now()))?;
    }
    let updated = db.get_watch(id)?.context("watch removido")?;
    Ok(CheckOutcome::Failed {
        watch: updated,
        error: error.to_string(),
        delay_secs: failure_delay_secs(failures, retry_after),
        notify_down,
    })
}

fn open_db(path: &Path) -> Result<Database> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Mode;
    use crate::spec::WatchSpec;
    use std::sync::OnceLock;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Variáveis globais de ambiente dos testes: banco/perfil só em diretório
    /// temporário (nunca `~/.curio`) e um "Chrome" de mentira para o modo js.
    fn test_env() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let dir = tempfile::tempdir().unwrap().keep();
            let script = dir.join("fake-chrome.sh");
            std::fs::write(
                &script,
                "#!/bin/sh\necho '<html><body><h1>Titulo</h1><p>montado por js</p></body></html>'\n",
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            std::env::set_var("CURIO_DB", dir.join("curio.db"));
            std::env::set_var("CURIO_CHROME", &script);
        });
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        path: PathBuf,
        db: Database,
        server: MockServer,
        client: Client,
        id: i64,
    }

    impl Fixture {
        async fn new(tweak: impl FnOnce(&mut WatchSpec)) -> Self {
            test_env();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("t.db");
            let db = Database::open_path(&path).unwrap();
            let server = MockServer::start().await;
            let mut spec = WatchSpec {
                url: format!("{}/p", server.uri()),
                interval_secs: 60,
                ..Default::default()
            };
            tweak(&mut spec);
            let id = db.insert_watch(&spec).unwrap().id;
            Self {
                _dir: dir,
                path,
                db,
                server,
                client: build_client(),
                id,
            }
        }

        async fn serve(&self, body: &str) {
            self.server.reset().await;
            Mock::given(method("GET"))
                .and(path("/p"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&self.server)
                .await;
        }

        async fn serve_status(&self, status: u16, retry_after: Option<&str>) {
            self.server.reset().await;
            let mut resp = ResponseTemplate::new(status);
            if let Some(v) = retry_after {
                resp = resp.insert_header("Retry-After", v);
            }
            Mock::given(method("GET"))
                .respond_with(resp)
                .mount(&self.server)
                .await;
        }

        async fn check(&self) -> CheckOutcome {
            check_watch(&self.client, &self.path, self.id, false).await
        }

        fn watch(&self) -> Watch {
            self.db.get_watch(self.id).unwrap().unwrap()
        }
    }

    fn ok(outcome: CheckOutcome) -> (Watch, bool, bool, bool) {
        match outcome {
            CheckOutcome::Ok {
                watch,
                changed,
                alert,
                recovered,
            } => (watch, changed, alert, recovered),
            CheckOutcome::Failed { error, .. } => panic!("falhou: {error}"),
            CheckOutcome::SkippedPaused => panic!("pausado"),
            CheckOutcome::Fatal(e) => panic!("fatal: {e}"),
        }
    }

    fn failed(outcome: CheckOutcome) -> (Watch, String, u64, bool) {
        match outcome {
            CheckOutcome::Failed {
                watch,
                error,
                delay_secs,
                notify_down,
            } => (watch, error, delay_secs, notify_down),
            CheckOutcome::Ok { .. } => panic!("deveria falhar"),
            CheckOutcome::SkippedPaused => panic!("pausado"),
            CheckOutcome::Fatal(e) => panic!("fatal: {e}"),
        }
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!(failure_delay_secs(1, None), 60);
        assert_eq!(failure_delay_secs(2, None), 120);
        assert_eq!(failure_delay_secs(3, None), 240);
        assert_eq!(failure_delay_secs(7, None), 3600);
        assert_eq!(failure_delay_secs(500, None), 3600);
    }

    #[test]
    fn retry_after_raises_but_never_lowers_the_delay() {
        assert_eq!(failure_delay_secs(1, Some(600)), 600);
        assert_eq!(failure_delay_secs(1, Some(5)), 60);
        assert_eq!(failure_delay_secs(8, Some(7200)), 7200);
    }

    #[tokio::test]
    async fn first_check_stores_snapshot_then_detects_change() {
        let f = Fixture::new(|_| {}).await;

        f.serve("<p>um</p><p>dois</p>").await;
        let (w, changed, alert, _) = ok(f.check().await);
        assert!(!changed && !alert, "primeira leitura não alerta");
        assert_eq!(w.last_status, WatchStatus::Ok);

        let (_, changed, _, _) = ok(f.check().await);
        assert!(!changed, "mesmo conteúdo, nada mudou");
        assert!(f.db.list_changes(f.id, 10).unwrap().is_empty());

        f.serve("<p>um</p><p>TRÊS</p>").await;
        let (w, changed, alert, _) = ok(f.check().await);
        assert!(changed && alert);
        assert_eq!(w.last_status, WatchStatus::Changed);
        assert!(w.unread_change);
        let changes = f.db.list_changes(f.id, 10).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(!changes[0].seen);
        assert!(changes[0].diff_text.contains("- dois"));
        assert!(changes[0].diff_text.contains("+ TRÊS"));
        assert!(changes[0].diff_text.contains("  um"), "traz contexto");
    }

    #[tokio::test]
    async fn failures_back_off_notify_once_and_recover() {
        let f = Fixture::new(|_| {}).await;
        f.serve("<p>ok</p>").await;
        ok(f.check().await);

        f.serve_status(500, None).await;
        let expected = [
            (60, false),
            (120, false),
            (240, true),
            (480, false),
            (960, false),
        ];
        for (i, (delay, notify)) in expected.iter().enumerate() {
            let (w, err, d, n) = failed(f.check().await);
            assert_eq!(err, "HTTP 500 Internal Server Error");
            assert_eq!((d, n), (*delay, *notify), "falha {}", i + 1);
            assert_eq!(w.consecutive_failures, i as u32 + 1);
            assert_eq!(w.last_status, WatchStatus::Error);
        }
        assert!(f.watch().down_since.is_some());

        f.serve("<p>ok</p>").await;
        let (w, changed, alert, recovered) = ok(f.check().await);
        assert!(recovered && !changed && !alert);
        assert_eq!(w.consecutive_failures, 0);
        assert!(w.down_since.is_none());

        let (_, _, _, recovered_again) = ok(f.check().await);
        assert!(!recovered_again, "recuperação é avisada uma única vez");
    }

    #[tokio::test]
    async fn downtime_is_not_renotified_after_recovery_cycle_resets() {
        let f = Fixture::new(|_| {}).await;
        f.serve_status(503, None).await;
        let notifications: Vec<bool> = {
            let mut v = Vec::new();
            for _ in 0..6 {
                v.push(failed(f.check().await).3);
            }
            v
        };
        assert_eq!(notifications, [false, false, true, false, false, false]);
    }

    #[tokio::test]
    async fn honors_retry_after_on_429() {
        let f = Fixture::new(|_| {}).await;
        f.serve_status(429, Some("600")).await;
        let (_, err, delay, _) = failed(f.check().await);
        assert_eq!(err, "HTTP 429 Too Many Requests");
        assert_eq!(delay, 600);

        f.serve_status(503, Some("5")).await;
        assert_eq!(
            failed(f.check().await).2,
            120,
            "Retry-After curto não encurta o backoff"
        );
    }

    #[tokio::test]
    async fn sends_headers_with_env_expansion() {
        std::env::set_var("CURIO_TEST_TOKEN_A", "s3cr3t");
        let f = Fixture::new(|s| {
            s.headers = "Authorization: Bearer ${CURIO_TEST_TOKEN_A}\nX-Extra: 1".into();
        })
        .await;
        Mock::given(method("GET"))
            .and(header("authorization", "Bearer s3cr3t"))
            .and(header("x-extra", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<p>autenticado</p>"))
            .expect(1)
            .mount(&f.server)
            .await;
        ok(f.check().await);
        let stored = f.watch();
        assert!(
            stored.headers.contains("${CURIO_TEST_TOKEN_A}") && !stored.headers.contains("s3cr3t"),
            "o banco guarda o modelo, nunca o segredo"
        );
        assert_eq!(
            f.db.latest_snapshot(f.id).unwrap().unwrap().normalized_text,
            "autenticado"
        );
    }

    #[tokio::test]
    async fn missing_env_var_fails_without_sending_the_request() {
        let f = Fixture::new(|s| {
            s.headers = "Authorization: Bearer ${CURIO_TEST_TOKEN_NAO_EXISTE}".into();
        })
        .await;
        f.serve("<p>x</p>").await;
        let (w, err, _, _) = failed(f.check().await);
        assert!(err.contains("CURIO_TEST_TOKEN_NAO_EXISTE"), "{err}");
        assert!(!err.contains("Bearer"));
        assert!(w
            .last_error
            .unwrap()
            .contains("CURIO_TEST_TOKEN_NAO_EXISTE"));
        assert!(f.server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn json_mode_flattens_and_diffs_per_leaf() {
        let f = Fixture::new(|s| {
            s.mode = Mode::Json;
            s.selector = "items[*].price".into();
        })
        .await;
        f.serve(r#"{"items":[{"price":10},{"price":20}],"ts":1}"#)
            .await;
        ok(f.check().await);
        assert_eq!(
            f.db.latest_snapshot(f.id).unwrap().unwrap().normalized_text,
            "items[0].price: 10\nitems[1].price: 20"
        );
        f.serve(r#"{"items":[{"price":10},{"price":18}],"ts":2}"#)
            .await;
        let (_, changed, alert, _) = ok(f.check().await);
        assert!(changed && alert);
        let diff = f.db.list_changes(f.id, 1).unwrap().remove(0).diff_text;
        assert!(diff.contains("- items[1].price: 20") && diff.contains("+ items[1].price: 18"));
    }

    #[tokio::test]
    async fn invalid_json_counts_as_a_failure() {
        let f = Fixture::new(|s| s.mode = Mode::Json).await;
        f.serve("<html>não sou json</html>").await;
        let (_, err, delay, _) = failed(f.check().await);
        assert!(err.contains("JSON"), "{err}");
        assert_eq!(delay, 60);
    }

    #[tokio::test]
    async fn condition_gates_alert_but_always_records_history() {
        let f = Fixture::new(|s| s.condition = "número < 100".into()).await;
        f.serve("<p>R$ 120</p>").await;
        ok(f.check().await);

        f.serve("<p>R$ 130</p>").await;
        let (w, changed, alert, _) = ok(f.check().await);
        assert!(changed && !alert, "130 não cruzou o limiar");
        assert!(!w.unread_change);
        assert_eq!(w.last_status, WatchStatus::Ok);

        f.serve("<p>R$ 90</p>").await;
        let (w, _, alert, _) = ok(f.check().await);
        assert!(alert, "130 → 90 cruza o limiar");
        assert!(w.unread_change);

        f.serve("<p>R$ 85</p>").await;
        let (_, changed, alert, _) = ok(f.check().await);
        assert!(changed && !alert, "continua abaixo: sem novo alerta");

        let changes = f.db.list_changes(f.id, 10).unwrap();
        assert_eq!(changes.len(), 3, "todas as mudanças ficam no histórico");
        assert_eq!(
            changes.iter().map(|c| c.seen).collect::<Vec<_>>(),
            vec![true, false, true]
        );
    }

    #[tokio::test]
    async fn paused_is_skipped_unless_forced() {
        let f = Fixture::new(|_| {}).await;
        f.serve("<p>x</p>").await;
        f.db.set_paused(f.id, true).unwrap();
        assert!(matches!(f.check().await, CheckOutcome::SkippedPaused));
        assert!(f.server.received_requests().await.unwrap().is_empty());

        let forced = check_watch(&f.client, &f.path, f.id, true).await;
        let (w, ..) = ok(forced);
        assert!(w.paused);
        assert_eq!(w.last_status, WatchStatus::Paused);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn js_mode_uses_the_browser_output() {
        let f = Fixture::new(|s| s.mode = Mode::Js).await;
        let (..) = ok(f.check().await);
        let snap = f.db.latest_snapshot(f.id).unwrap().unwrap();
        assert!(snap.normalized_text.contains("montado por js"));
        assert!(
            f.server.received_requests().await.unwrap().is_empty(),
            "o modo js não faz GET direto"
        );
    }

    #[tokio::test]
    async fn selector_without_match_is_a_failure_not_a_panic() {
        let f = Fixture::new(|s| s.selector = "#nao-existe".into()).await;
        f.serve("<p>x</p>").await;
        let (w, err, ..) = failed(f.check().await);
        assert!(err.contains("seletor"), "{err}");
        assert_eq!(w.consecutive_failures, 1);
    }
}
