use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

fn directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rivals-diag-{}-{}",
        std::process::id(),
        new_session()
    ));
    fs::create_dir_all(path.join("Logs")).unwrap();
    path
}

fn record() -> Record {
    serde_json::from_str(r#"{"schema":1,"id":"session-1","timestamp":1789296000,"client_version":"test-1","build":"abc","kind":"lua_error","component":"MobaMinimap","message":"nil value","context":"map=901 minimap=260x260","trace":"MobaMinimap.lua:42"}"#).unwrap()
}

#[test]
fn events_preserve_client_version_and_group_errors_without_player_identity() {
    let event = event(&record());
    assert_eq!(event["release"], "rivals-beyond-client@test-1");
    assert_eq!(event["tags"]["client_build"], "abc");
    assert_eq!(event["extra"]["ui_context"], "map=901 minimap=260x260");
    assert!(event.get("user").is_none());
    assert_eq!(event["event_id"], super::event(&record())["event_id"]);
    assert_eq!(event["exception"]["values"][0]["type"], "LuaError");
}

#[test]
fn transport_only_accepts_known_bounded_records_and_complete_lines() {
    let mut r = record();
    assert!(r.valid());
    r.kind = "chat".into();
    assert!(!r.valid());
    r = record();
    r.message = "x".repeat(2049);
    assert!(!r.valid());
    r = record();
    r.schema = 2;
    assert!(!r.valid());
    let root = directory();
    let line = serde_json::to_string(&record()).unwrap();
    fs::write(root.join(LOG), format!("{}\n{{\"incomplete", line)).unwrap();
    assert_eq!(read_records(&root).len(), 1);
    fs::write(root.join(LOG), "x".repeat(MAX_FILE_BYTES as usize + 1)).unwrap();
    assert!(read_records(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reports_scrub_emails_urls_credentials_and_local_user_paths() {
    let text = scrub("error user@example.com https://x.test/?token=secret C:\\Users\\Alice\\file token=abc password hunter2 Interface\\FrameXML\\MobaUI.lua:42");
    for private in ["user@example", "x.test", "Alice", "abc", "hunter2"] {
        assert!(!text.contains(private));
    }
    assert!(text.contains("MobaUI.lua:42"));
}

#[test]
fn only_native_warning_and_error_lines_are_collected_once_for_the_current_launch() {
    let root = directory();
    let mut session = record();
    session.id = format!("{}-start", new_session());
    session.timestamp = now();
    fs::write(
        root.join("Logs/wxl-core.log"),
        "INFO: started\nplayer chat\n[12:00:00] ERROR: outline failed\n[12:00:01] ERROR: outline failed\nWARN: texture missing\n",
    )
    .unwrap();
    let mut seen = Vec::new();
    collect_native_errors(&root, &session, &mut seen);
    collect_native_errors(&root, &session, &mut seen);
    let records = read_records(&root);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].kind, "native_error");
    assert_eq!(records[1].kind, "native_warning");
    assert!(records
        .iter()
        .all(|r| r.client_version == session.client_version));
    assert!(!records.iter().any(|r| r.message.contains("chat")));
    session.timestamp += 100;
    collect_native_errors(&root, &session, &mut Vec::new());
    assert_eq!(
        read_records(&root).len(),
        2,
        "Never attribute a stale native log to a later launch"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_long_native_log_keeps_its_recent_errors_without_reading_the_entire_file() {
    let root = directory();
    let mut session = record();
    session.id = format!("{}-start", new_session());
    session.timestamp = now();
    fs::write(
        root.join("Logs/wxl-core.log"),
        format!(
            "{}\nERROR: recent failure\n",
            "INFO: old\n".repeat(MAX_FILE_BYTES as usize)
        ),
    )
    .unwrap();
    collect_native_errors(&root, &session, &mut Vec::new());
    assert_eq!(read_records(&root).len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sentry_quota_headers_pause_even_successful_delivery_and_hourly_limits_keep_backlog_local() {
    let root = directory();
    append_record(&root, &record()).unwrap();
    let (url, join) = server("200 OK\r\nX-Sentry-Rate-Limits: 90:error:organization:quota");
    let mut relay = Relay::new(&root).unwrap();
    relay.endpoint = url;
    relay.flush(1000);
    join.join().unwrap();
    assert_eq!(relay.state.retry_at, 1090);
    assert_eq!(relay.state.sent.len(), 1);
    let mut another = record();
    another.id = "session-2".into();
    append_record(&root, &another).unwrap();
    relay.endpoint = "http://127.0.0.1:1".into();
    relay.state.sent_this_hour = MAX_HOURLY_EVENTS;
    relay.flush(1100);
    assert_eq!(relay.state.sent.len(), 1);
    assert_eq!(
        relay.state.retry_at, 1090,
        "The per-hour cap must prevent a network attempt"
    );
    drop(relay);
    fs::remove_dir_all(root).unwrap();
}

fn server(status: &str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/1/envelope/", listener.local_addr().unwrap());
    let status = status.to_owned();
    let join = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut data = Vec::new();
        let mut buf = [0; 4096];
        loop {
            let n = socket.read(&mut buf).unwrap();
            assert!(n > 0);
            data.extend_from_slice(&buf[..n]);
            if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&data[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .unwrap()
                    .parse()
                    .unwrap();
                if data.len() >= end + 4 + length {
                    break;
                }
            }
        }
        write!(
            socket,
            "HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .unwrap();
        String::from_utf8(data).unwrap()
    });
    (url, join)
}

#[test]
fn successful_delivery_is_persisted_and_failed_delivery_retries_after_backoff() {
    let root = directory();
    append_record(&root, &record()).unwrap();
    let (url, join) = server("429 Too Many Requests\r\nRetry-After: 120");
    let mut relay = Relay::new(&root).unwrap();
    relay.endpoint = url;
    relay.flush(1000);
    join.join().unwrap();
    assert!(relay.state.sent.is_empty());
    assert!(relay.state.retry_at >= 1120);
    let (url, join) = server("200 OK");
    relay.endpoint = url;
    relay.flush(1120);
    let request = join.join().unwrap();
    assert!(request.contains("application/x-sentry-envelope"));
    assert!(request.contains("rivals-beyond-client@test-1"));
    assert_eq!(relay.state.sent.len(), 1);
    drop(relay);
    let mut reloaded = Relay::new(&root).unwrap();
    assert_eq!(reloaded.state.sent.len(), 1);
    reloaded.endpoint = "http://127.0.0.1:1".into();
    reloaded.flush(2000);
    assert_eq!(
        reloaded.state.retry_at, 0,
        "Already delivered events must not be posted again"
    );
    drop(reloaded);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn opted_out_records_are_never_uploaded_after_opt_in() {
    let root = directory();
    let mut record = record();
    record.upload = false;
    append_record(&root, &record).unwrap();
    let mut relay = Relay::new(&root).unwrap();
    relay.endpoint = "http://127.0.0.1:1".into();
    relay.flush(1000);
    assert!(relay.state.sent.is_empty());
    assert_eq!(relay.state.retry_at, 0);
    drop(relay);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn every_started_process_records_its_version_duration_and_abnormal_exit_without_network_when_disabled(
) {
    let root = directory();
    let manifest = crate::updater::Manifest {
        locales: Vec::new(),
        schema_version: 1,
        sequence: 42,
        client_version: "test-42".into(),
        object_base_url: String::new(),
        file_count: 0,
        total_size: 0,
        files: vec![],
    };
    for code in [0, 7] {
        let mut child = std::process::Command::new("sh")
            .args(["-c", &format!("exit {code}")])
            .spawn()
            .unwrap();
        let resolution =
            crate::updater::configure_client_defaults(&root, Some((2560, 1440))).unwrap();
        let status = watch_game(
            &mut child,
            &root,
            &manifest,
            false,
            &new_session(),
            &resolution,
        )
        .unwrap();
        assert_eq!(status.code(), Some(code));
    }
    let records = read_records(&root);
    assert_eq!(records.len(), 4);
    assert!(records[0]
        .context
        .contains("monitor=2560x1440 resolution_configured=2560x1440 resolution_source=monitor"));
    assert!(records[2].context.contains("resolution_source=saved"));
    let launch = event(&records[0]);
    assert_eq!(launch["tags"]["monitor"], "2560x1440");
    assert_eq!(launch["tags"]["resolution_configured"], "2560x1440");
    assert_eq!(launch["tags"]["resolution_source"], "monitor");
    assert_eq!(launch["tags"]["hw_detect_disabled"], "true");
    let mut in_game = record();
    in_game.context = "resolution=1920x1080".into();
    assert_eq!(event(&in_game)["tags"]["resolution"], "1920x1080");
    assert_eq!(
        records.iter().filter(|r| r.kind == "game_started").count(),
        2
    );
    assert!(records
        .iter()
        .all(|r| r.client_version == "test-42" && !r.upload));
    assert!(
        records[3].kind == "client_crash" && records[3].context.contains("exit_code=0x00000007")
    );
    assert!(records[3].context.contains("duration_seconds="));
    assert!(!root.join(STATE).exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn only_one_relay_can_consume_a_clients_journal_at_a_time() {
    let root = directory();
    let first = Relay::new(&root).unwrap();
    assert!(Relay::new(&root).is_err());
    drop(first);
    assert!(Relay::new(&root).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn uploader_ignores_links_outside_the_client_log_directory() {
    let root = directory();
    let outside = directory();
    append_record(&outside, &record()).unwrap();
    std::os::unix::fs::symlink(outside.join(LOG), root.join(LOG)).unwrap();
    assert!(read_records(&root).is_empty());
    fs::remove_file(root.join(LOG)).unwrap();
    fs::remove_dir(root.join("Logs")).unwrap();
    std::os::unix::fs::symlink(outside.join("Logs"), root.join("Logs")).unwrap();
    assert!(Relay::new(&root).is_err());
    assert!(read_records(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn journal_rotates_and_successful_launches_have_distinct_session_ids() {
    let root = directory();
    fs::write(root.join(LOG), "x".repeat(MAX_FILE_BYTES as usize)).unwrap();
    append_record(&root, &record()).unwrap();
    assert!(root.join(PREVIOUS_LOG).exists());
    assert_eq!(read_records(&root).len(), 1);
    assert_ne!(new_session(), new_session());
    fs::remove_dir_all(root).unwrap();
}
