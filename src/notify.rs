//! System notifications for Curió.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use notify_rust::Notification;

/// `show()` pode bloquear (D-Bus / osascript): roda fora do runtime.
fn spawn_notification(summary: String, body: String, timeout_ms: i32) {
    std::thread::spawn(move || {
        let _ = Notification::new()
            .summary(&summary)
            .body(&body)
            .appname("Curió")
            .timeout(timeout_ms)
            .show();
    });
}

pub fn notify_change(label: &str, url: &str) {
    let summary = "Curió — mudança detectada".to_string();
    let body = if label.is_empty() {
        format!("{url}")
    } else {
        format!("{label}\n{url}")
    };
    spawn_notification(summary, body, 8000);
}

pub fn notify_downtime(label: &str, url: &str, error: &str) {
    let summary = "Curió — falha ao monitorar";
    let name = if label.is_empty() { url } else { label };
    let body = format!("{name}\n{error}");
    spawn_notification(summary.to_string(), body, 10000);
}
