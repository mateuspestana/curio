//! System notifications for Curió.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use notify_rust::Notification;

pub fn notify_change(label: &str, url: &str) {
    let summary = format!("Curió — mudança detectada");
    let body = if label.is_empty() {
        format!("{url}")
    } else {
        format!("{label}\n{url}")
    };
    let _ = Notification::new()
        .summary(&summary)
        .body(&body)
        .appname("Curió")
        .timeout(8000)
        .show();
}

pub fn notify_downtime(label: &str, url: &str, error: &str) {
    let summary = "Curió — falha ao monitorar";
    let name = if label.is_empty() { url } else { label };
    let body = format!("{name}\n{error}");
    let _ = Notification::new()
        .summary(summary)
        .body(&body)
        .appname("Curió")
        .timeout(10000)
        .show();
}
