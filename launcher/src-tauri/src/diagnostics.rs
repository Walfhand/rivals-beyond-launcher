//! Bounded relay for game diagnostics using Sentry's documented envelope ingestion API.
//! The game writes locally; HTTPS and retry bookkeeping never run on its render thread.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DSN: &str = "https://852918cc0856586121415e03184d42a4@o4512077726285824.ingest.de.sentry.io/4512077733036112";
const LOG: &str = "Logs/RivalsDiagnostics.jsonl";
const PREVIOUS_LOG: &str = "Logs/RivalsDiagnostics.previous.jsonl";
const STATE: &str = "Logs/RivalsDiagnostics.sent.json";
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_SENT_IDS: usize = 2048;
const MAX_BATCH: usize = 4;
const MAX_HOURLY_EVENTS: u32 = 120;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: u32,
    pub id: String,
    pub timestamp: u64,
    pub client_version: String,
    pub build: String,
    pub kind: String,
    pub component: String,
    pub message: String,
    pub context: String,
    pub trace: String,
    #[serde(default = "default_upload")]
    pub upload: bool,
}

fn default_upload() -> bool {
    true
}

impl Record {
    fn valid(&self) -> bool {
        self.schema == 1
            && !self.id.is_empty()
            && self.id.len() <= 64
            && self.client_version.len() <= 64
            && self.build.len() <= 64
            && self.component.len() <= 64
            && self.message.len() <= 2048
            && self.context.len() <= 1024
            && self.trace.len() <= 2048
            && matches!(
                self.kind.as_str(),
                "lua_error"
                    | "diagnostic"
                    | "game_started"
                    | "game_exited"
                    | "client_crash"
                    | "launcher_error"
                    | "native_error"
                    | "native_warning"
            )
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn new_session() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let seed = format!(
        "{}-{:?}-{}",
        std::process::id(),
        SystemTime::now(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    format!("{:x}", Sha256::digest(seed.as_bytes()))[..24].to_owned()
}

fn bounded_read(path: &Path, maximum: u64) -> std::io::Result<Vec<u8>> {
    // Only our fixed filenames are read; never follow a player-created symlink to another file.
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || crate::updater::is_link_or_reparse(&metadata) {
        return Ok(Vec::new());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(maximum + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        bytes.clear();
    }
    Ok(bytes)
}

fn log_directory(root: &Path) -> std::io::Result<()> {
    let logs = root.join("Logs");
    if !logs.exists() {
        fs::create_dir(&logs)?;
    }
    let metadata = fs::symlink_metadata(logs)?;
    if !metadata.is_dir() || crate::updater::is_link_or_reparse(&metadata) {
        return Err(std::io::Error::other("Invalid diagnostic directory"));
    }
    Ok(())
}

fn read_records(root: &Path) -> Vec<Record> {
    let mut records = Vec::new();
    if log_directory(root).is_err() {
        return records;
    }
    for name in [PREVIOUS_LOG, LOG] {
        let bytes = bounded_read(&root.join(name), MAX_FILE_BYTES).unwrap_or_default();
        for line in bytes.split_inclusive(|&b| b == b'\n') {
            if line.last() != Some(&b'\n') || line.len() > 32 * 1024 {
                continue;
            }
            if let Ok(record) = serde_json::from_slice::<Record>(line) {
                if record.valid() {
                    records.push(record);
                }
            }
        }
    }
    records
}

pub fn append_record(root: &Path, record: &Record) -> std::io::Result<()> {
    if !record.valid() {
        return Err(std::io::Error::other("Invalid diagnostic record"));
    }
    log_directory(root)?;
    let path = root.join(LOG);
    if fs::symlink_metadata(&path)
        .is_ok_and(|m| !m.file_type().is_file() || crate::updater::is_link_or_reparse(&m))
    {
        return Err(std::io::Error::other("Invalid diagnostic journal"));
    }
    let mut bytes = serde_json::to_vec(record)?;
    bytes.push(b'\n');
    if fs::metadata(&path).is_ok_and(|m| m.len() + bytes.len() as u64 > MAX_FILE_BYTES) {
        let previous = root.join(PREVIOUS_LOG);
        if previous.exists() {
            fs::remove_file(&previous)?;
        }
        fs::rename(&path, previous)?;
    }
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(&bytes)
}

// Do not upload arbitrary log folders, chat, account configuration or crash memory dumps.
// Error strings can still mention paths/URLs; scrub these before serializing the envelope.
fn scrub(text: &str) -> String {
    let mut hide_next = false;
    text.lines()
        .map(|line| {
            line.split_whitespace()
                .map(|word| {
                    let lower = word.to_ascii_lowercase();
                    let sensitive = ["password", "passwd", "token", "authorization", "secret"]
                        .iter()
                        .any(|key| lower.contains(key));
                    let hidden = hide_next
                        || sensitive
                        || word.contains('@')
                        || word.contains("://")
                        || word.contains(":\\")
                        || word.contains(":/")
                        || word.contains("/home/")
                        || word.contains("/Users/");
                    hide_next = sensitive && !word.contains('=');
                    if hidden {
                        "[filtered]"
                    } else {
                        word
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn event(record: &Record) -> Value {
    let identity = format!("{}:{}", record.build, record.id);
    let id = format!("{:x}", Sha256::digest(identity.as_bytes()))[..32].to_owned();
    let message = scrub(&record.message);
    let error = matches!(
        record.kind.as_str(),
        "lua_error" | "client_crash" | "launcher_error" | "native_error"
    );
    let mut event = json!({
        "event_id": id, "timestamp": record.timestamp, "platform": "other",
        "level": if error { "error" } else if record.kind == "native_warning" { "warning" } else { "info" }, "logger": "rivals.client",
        "release": format!("rivals-beyond-client@{}", scrub(&record.client_version)),
        "environment": if record.client_version == "local" { "development" } else { "production" },
        "message": message,
        "fingerprint": [record.kind, scrub(&record.component), message],
        "tags": {"kind": record.kind, "component": scrub(&record.component),
            "session": record.id.rsplit_once('-').map(|(session, _)| session).unwrap_or(&record.id),
            "client_build": scrub(&record.build), "client_version": scrub(&record.client_version),
            "launcher_version": env!("CARGO_PKG_VERSION")},
        "extra": {"ui_context": scrub(&record.context), "lua_trace": scrub(&record.trace)}
    });
    if error {
        event["exception"] = json!({"values": [{"type": if record.kind == "lua_error" { "LuaError" } else { "ClientError" }, "value": message}]});
    }
    for (key, value) in record
        .context
        .split_whitespace()
        .filter_map(|part| part.split_once('='))
    {
        if matches!(
            key,
            "backend"
                | "os"
                | "patch"
                | "outline"
                | "modern_m2"
                | "duration_seconds"
                | "exit_code"
                | "map"
                | "renderer"
                | "multisample"
                | "resolution"
                | "monitor"
                | "resolution_configured"
                | "resolution_source"
                | "hw_detect_disabled"
        ) {
            event["tags"][key] = json!(scrub(value));
        }
    }
    event
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct DeliveryState {
    sent: VecDeque<String>,
    retry_at: u64,
    hour: u64,
    sent_this_hour: u32,
}

pub struct Relay {
    root: PathBuf,
    http: reqwest::blocking::Client,
    endpoint: String,
    state: DeliveryState,
    _lock: fs::File,
}

impl Relay {
    pub fn new(root: &Path) -> Result<Self, String> {
        log_directory(root).map_err(|e| e.to_string())?;
        let lock_path = root.join("Logs/RivalsDiagnostics.lock");
        if fs::symlink_metadata(&lock_path)
            .is_ok_and(|m| !m.is_file() || crate::updater::is_link_or_reparse(&m))
        {
            return Err("Invalid diagnostic lock".into());
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|e| e.to_string())?;
        lock.try_lock().map_err(|e| e.to_string())?;
        // Exclusive ownership means a leftover temporary state can only be from an interrupted save.
        let _ = fs::remove_file(root.join(format!("{STATE}.tmp")));
        let dsn = reqwest::Url::parse(DSN).map_err(|e| e.to_string())?;
        let project = dsn.path().trim_start_matches('/');
        let endpoint = format!(
            "https://{}/api/{}/envelope/",
            dsn.host_str().ok_or("Missing Sentry host")?,
            project
        );
        let mut state: DeliveryState = serde_json::from_slice(
            &bounded_read(&root.join(STATE), MAX_FILE_BYTES).unwrap_or_default(),
        )
        .unwrap_or_default();
        state
            .sent
            .retain(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()));
        while state.sent.len() > MAX_SENT_IDS {
            state.sent.pop_front();
        }
        Ok(Self {
            root: root.to_owned(),
            endpoint,
            state,
            _lock: lock,
            http: reqwest::blocking::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())?,
        })
    }

    fn save(&self) -> std::io::Result<()> {
        log_directory(&self.root)?;
        let temporary = self.root.join(format!("{STATE}.tmp"));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(&serde_json::to_vec(&self.state)?)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, self.root.join(STATE))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn flush(&mut self, timestamp: u64) {
        if timestamp < self.state.retry_at {
            return;
        }
        if self.state.hour != timestamp / 3600 {
            self.state.hour = timestamp / 3600;
            self.state.sent_this_hour = 0;
        }
        let mut attempted = 0;
        for record in read_records(&self.root).into_iter().rev() {
            let event = event(&record);
            let id = event["event_id"].as_str().unwrap();
            if !record.upload || self.state.sent.iter().any(|sent| sent == id) {
                continue;
            }
            if attempted >= MAX_BATCH || self.state.sent_this_hour >= MAX_HOURLY_EVENTS {
                break;
            }
            attempted += 1;
            let body = format!(
                "{}\n{}\n{}\n",
                json!({"event_id": id, "dsn": DSN}),
                json!({"type":"event"}),
                event
            );
            let response = self
                .http
                .post(&self.endpoint)
                .header("Content-Type", "application/x-sentry-envelope")
                .body(body)
                .send();
            let mut backoff = 0;
            let success = if let Ok(response) = response {
                if let Some(value) = response
                    .headers()
                    .get("x-sentry-rate-limits")
                    .and_then(|h| h.to_str().ok())
                {
                    for limit in value.split(',') {
                        let fields: Vec<_> = limit.trim().split(':').collect();
                        if fields.len() >= 2
                            && (fields[1].is_empty() || fields[1].split(';').any(|c| c == "error"))
                        {
                            backoff =
                                backoff.max(fields[0].parse::<f64>().unwrap_or(60.0).ceil() as u64);
                        }
                    }
                }
                if !response.status().is_success() {
                    backoff = backoff.max(
                        response
                            .headers()
                            .get("retry-after")
                            .and_then(|h| h.to_str().ok())
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(60),
                    );
                }
                response.status().is_success()
            } else {
                backoff = 60;
                false
            };
            if success {
                self.state.sent.push_back(id.to_owned());
                self.state.sent_this_hour += 1;
                while self.state.sent.len() > MAX_SENT_IDS {
                    self.state.sent.pop_front();
                }
            }
            self.state.retry_at = if backoff > 0 {
                timestamp.saturating_add(backoff)
            } else {
                0
            };
            if self.save().is_err() || !success || backoff > 0 {
                break;
            }
        }
    }
}

pub fn watch_game(
    child: &mut std::process::Child,
    root: &Path,
    manifest: &crate::updater::Manifest,
    enabled: bool,
    session: &str,
    resolution: &crate::updater::ResolutionChoice,
) -> std::io::Result<std::process::ExitStatus> {
    let started = std::time::Instant::now();
    let hash = |path: &str| {
        manifest
            .files
            .iter()
            .find(|file| file.path.eq_ignore_ascii_case(path))
            .map(|file| file.sha256.as_str())
            .unwrap_or("unknown")
    };
    let dimensions = |size: Option<(u32, u32)>| {
        size.map(|(w, h)| format!("{w}x{h}"))
            .unwrap_or_else(|| "unknown".into())
    };
    let context = format!(
        "os={} backend={} patch={} outline={} modern_m2={} manifest_sequence={} monitor={} resolution_configured={} resolution_source={} hw_detect_disabled={}",
        std::env::consts::OS,
        if root.join("d3d9.dll").is_file() {
            "dxvk"
        } else {
            "d3d9"
        },
        hash("Data/PATCH-Z.MPQ"),
        hash("Extensions/UnitOutline/UnitOutline.dll"),
        hash("Extensions/wxl-modern-m2/wxl-modern-m2.dll"),
        manifest.sequence,
        dimensions(resolution.monitor),
        dimensions(resolution.configured),
        resolution.source,
        resolution.hardware_detection_disabled
    );
    let mut record = Record {
        schema: 1,
        id: format!("{session}-start"),
        timestamp: now(),
        client_version: manifest.client_version.clone(),
        build: hash("Extensions/RivalsBeyond/RivalsBeyond.dll").into(),
        kind: "game_started".into(),
        component: "launcher".into(),
        message: "Game started".into(),
        context,
        trace: String::new(),
        upload: enabled,
    };
    let _ = append_record(root, &record);
    let stop_relay = if enabled {
        Some(start_relay(root.to_owned()))
    } else {
        None
    };
    let mut next_send = 0;
    let mut native_seen = Vec::new();
    loop {
        if let Some(status) = child.try_wait()? {
            collect_native_errors(root, &record, &mut native_seen);
            record.id = format!("{session}-exit");
            record.timestamp = now();
            record.kind = if status.success() {
                "game_exited"
            } else {
                "client_crash"
            }
            .into();
            record.message = if status.success() {
                "Game exited"
            } else {
                "Game exited abnormally"
            }
            .into();
            record.context.push_str(&format!(
                " duration_seconds={} exit_code={}",
                started.elapsed().as_secs(),
                status
                    .code()
                    .map(|c| format!("0x{:08x}", c as u32))
                    .unwrap_or_else(|| "signal".into())
            ));
            let _ = append_record(root, &record);
            if let Some(stop) = stop_relay {
                let _ = stop.send(());
            }
            return Ok(status);
        }
        if now() >= next_send {
            collect_native_errors(root, &record, &mut native_seen);
            next_send = now().saturating_add(30);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn start_relay(root: PathBuf) -> std::sync::mpsc::Sender<()> {
    let (stop, receiver) = std::sync::mpsc::channel();
    // A slow ingestion endpoint must not delay game exit or the launcher's controls.
    std::thread::spawn(move || {
        let mut relay = None;
        loop {
            if relay.is_none() {
                relay = Relay::new(&root).ok();
            }
            if let Some(relay) = &mut relay {
                relay.flush(now());
            }
            match receiver.recv_timeout(Duration::from_secs(30)) {
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
                _ => {
                    if let Some(relay) = &mut relay {
                        relay.flush(now());
                    }
                    break;
                }
            }
        }
    });
    stop
}

fn collect_native_errors(root: &Path, session: &Record, seen: &mut Vec<String>) {
    if seen.len() >= 20 || log_directory(root).is_err() {
        return;
    }
    let path = root.join("Logs/wxl-core.log");
    let modified = fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|t| t.as_secs())
        .unwrap_or(0);
    if modified < session.timestamp {
        return;
    }
    let bytes = (|| -> std::io::Result<Vec<u8>> {
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || crate::updater::is_link_or_reparse(&metadata) {
            return Ok(Vec::new());
        }
        let mut file = fs::File::open(&path)?;
        let start = file.metadata()?.len().saturating_sub(MAX_FILE_BYTES);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES).read_to_end(&mut bytes)?;
        if start > 0 {
            let first_line = bytes
                .iter()
                .position(|&b| b == b'\n')
                .map(|i| i + 1)
                .unwrap_or(bytes.len());
            bytes.drain(..first_line);
        }
        Ok(bytes)
    })()
    .unwrap_or_default();
    for line in String::from_utf8_lossy(&bytes).lines() {
        let kind = if line.contains("ERROR:") {
            "native_error"
        } else if line.contains("WARN:") {
            "native_warning"
        } else {
            continue;
        };
        let marker = if kind == "native_error" {
            "ERROR:"
        } else {
            "WARN:"
        };
        let line = line
            .split_once(marker)
            .map(|(_, text)| text.trim())
            .unwrap_or(line);
        let hash = format!("{:x}", Sha256::digest(line.as_bytes()));
        if seen.iter().any(|s| s == &hash) {
            continue;
        }
        let mut length = line.len().min(2048);
        while !line.is_char_boundary(length) {
            length -= 1;
        }
        let record = Record {
            schema: 1,
            id: format!("{}-{}", &session.id[..24], &hash[..24]),
            timestamp: now(),
            client_version: session.client_version.clone(),
            build: session.build.clone(),
            kind: kind.into(),
            component: "WarcraftXL".into(),
            message: line[..length].into(),
            context: session.context.clone(),
            trace: String::new(),
            upload: session.upload,
        };
        if append_record(root, &record).is_ok() {
            seen.push(hash);
        }
        if seen.len() >= 20 {
            break;
        }
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
