//! Explicit smoke test: sends one synthetic development event to the configured Sentry project.
use moba_launcher_core::diagnostics::{self, Record, Relay};

fn main() {
    assert_eq!(
        std::env::args().nth(1).as_deref(),
        Some("--send"),
        "Pass --send to send the synthetic event"
    );
    let root = std::env::temp_dir().join(format!(
        "rivals-sentry-smoke-{}",
        diagnostics::new_session()
    ));
    std::fs::create_dir(&root).unwrap();
    let record = Record {
        schema: 1,
        id: diagnostics::new_session(),
        timestamp: diagnostics::now(),
        client_version: "local".into(),
        build: "integration-test".into(),
        kind: "diagnostic".into(),
        component: "integration_test".into(),
        message: "Rivals Beyond diagnostic integration test".into(),
        context: "map=901 renderer=d3d9 source=synthetic".into(),
        trace: String::new(),
        upload: true,
    };
    diagnostics::append_record(&root, &record).unwrap();
    let mut relay = Relay::new(&root).unwrap();
    relay.flush(diagnostics::now());
    drop(relay);
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("Logs/RivalsDiagnostics.sent.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        state["sent"].as_array().unwrap().len(),
        1,
        "Sentry has not acknowledged the event; journal kept in {}",
        root.display()
    );
    println!(
        "Sentry accepted event {} (development / integration_test)",
        state["sent"][0]
    );
    std::fs::remove_dir_all(root).unwrap();
}
