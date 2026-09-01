#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use moba_launcher_core::{news, updater, LauncherError};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::{Update, UpdaterExt};

const MANIFEST_URL: &str = match option_env!("MOBA_MANIFEST_URL") {
    Some(value) => value,
    None => "https://moba-data.nbg1.your-objectstorage.com/client/manifests/stable.json",
};
const NEWS_URL: &str = match option_env!("MOBA_NEWS_URL") {
    Some(value) => value,
    None => "https://moba-data.nbg1.your-objectstorage.com/launcher/news/stable.json",
};
const REALM_ADDRESS: &str = match option_env!("MOBA_REALM_ADDRESS") {
    Some(value) => value,
    None => "127.0.0.1",
};

struct Busy(AtomicBool);

#[derive(Default)]
struct PendingLauncherUpdate(Mutex<Option<Update>>);

#[derive(Serialize)]
struct LauncherUpdateStatus {
    current_version: String,
    version: String,
    notes: Option<String>,
}

#[derive(Clone, Serialize)]
struct LauncherUpdateProgress {
    downloaded: u64,
    total: Option<u64>,
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("MobaLauncher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(None)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Initialisation réseau impossible : {error}"))
}

#[tauri::command]
fn choose_client_dir(app: tauri::AppHandle) -> Result<Option<String>, LauncherError> {
    app.dialog()
        .file()
        .set_title("Choisir le dossier du client MOBA")
        .blocking_pick_folder()
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|_| "Le dossier choisi n’est pas un chemin local.".to_string())
        })
        .transpose()
        .map_err(LauncherError::from)
}

#[tauri::command]
async fn client_status(client_dir: String) -> Result<updater::ClientStatus, LauncherError> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let http = http_client()?;
        let loaded = updater::fetch_manifest(&http, MANIFEST_URL, updater::public_key()?)?;
        updater::client_status_against_manifest(Path::new(&client_dir), &loaded)
    })
    .await
    .map_err(|error| format!("La vérification du client a échoué : {error}"))?;
    result.map_err(LauncherError::from)
}

#[tauri::command]
async fn launcher_news() -> Result<news::NewsFeed, LauncherError> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let http = http_client()?;
        news::fetch_news(&http, NEWS_URL, updater::public_key()?)
    })
    .await
    .map_err(|error| format!("La récupération des nouveautés a échoué : {error}"))?;
    result.map_err(LauncherError::from)
}

fn launcher_updater_error(error: tauri_plugin_updater::Error) -> LauncherError {
    use tauri_plugin_updater::Error;

    let detail = error.to_string();
    match error {
        Error::Minisign(_)
        | Error::Base64(_)
        | Error::SignatureUtf8(_)
        | Error::InvalidUpdaterFormat
        | Error::InsecureTransportProtocol => LauncherError {
            code: "security",
            message: "La mise à jour du launcher n’a pas pu être authentifiée.",
            detail,
            retryable: false,
        },
        Error::Reqwest(_) | Error::Network(_) | Error::ReleaseNotFound => LauncherError {
            code: "network",
            message: "Connexion au service de mise à jour du launcher interrompue.",
            detail,
            retryable: true,
        },
        _ => LauncherError {
            code: "launcher_update",
            message: "La mise à jour du launcher a échoué.",
            detail,
            retryable: true,
        },
    }
}

#[tauri::command]
async fn check_launcher_update(
    app: tauri::AppHandle,
    pending: tauri::State<'_, PendingLauncherUpdate>,
) -> Result<Option<LauncherUpdateStatus>, LauncherError> {
    let update = app
        .updater()
        .map_err(launcher_updater_error)?
        .check()
        .await
        .map_err(launcher_updater_error)?;
    let status = update.as_ref().map(|update| LauncherUpdateStatus {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        notes: update.body.clone(),
    });
    *pending.0.lock().map_err(|error| LauncherError {
        code: "launcher_update",
        message: "L’état de mise à jour du launcher est indisponible.",
        detail: error.to_string(),
        retryable: true,
    })? = update;
    Ok(status)
}

#[tauri::command]
async fn install_launcher_update(
    app: tauri::AppHandle,
    pending: tauri::State<'_, PendingLauncherUpdate>,
) -> Result<(), LauncherError> {
    if app.state::<Busy>().0.swap(true, Ordering::AcqRel) {
        return Err(LauncherError::from(
            "Une opération est déjà en cours.".to_string(),
        ));
    }
    let update = pending.0.lock().map_err(|error| LauncherError {
        code: "launcher_update",
        message: "L’état de mise à jour du launcher est indisponible.",
        detail: error.to_string(),
        retryable: true,
    });
    let update = match update.and_then(|mut pending| {
        pending.take().ok_or_else(|| LauncherError {
            code: "launcher_update",
            message: "Aucune mise à jour du launcher n’est prête.",
            detail: "La vérification doit être relancée.".into(),
            retryable: true,
        })
    }) {
        Ok(update) => update,
        Err(error) => {
            app.state::<Busy>().0.store(false, Ordering::Release);
            return Err(error);
        }
    };
    let progress_app = app.clone();
    let finished_app = app.clone();
    let mut downloaded = 0u64;
    let result = update
        .download_and_install(
            move |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                let _ = progress_app.emit(
                    "launcher-self-update-progress",
                    LauncherUpdateProgress { downloaded, total },
                );
            },
            move || {
                let _ = finished_app.emit("launcher-self-update-downloaded", ());
            },
        )
        .await
        .map_err(launcher_updater_error);
    if let Err(error) = result {
        app.state::<Busy>().0.store(false, Ordering::Release);
        return Err(error);
    }
    app.restart()
}

#[tauri::command]
async fn update_client(
    app: tauri::AppHandle,
    client_dir: String,
    repair: bool,
) -> Result<updater::UpdateSummary, LauncherError> {
    if app.state::<Busy>().0.swap(true, Ordering::AcqRel) {
        return Err(LauncherError::from(
            "Une opération est déjà en cours.".to_string(),
        ));
    }
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let http = http_client()?;
        let loaded = updater::fetch_manifest(&http, MANIFEST_URL, updater::public_key()?)?;
        updater::update_client(
            &http,
            Path::new(&client_dir),
            loaded,
            REALM_ADDRESS,
            repair,
            |progress| {
                let _ = worker_app.emit("launcher-progress", progress);
            },
        )
    })
    .await
    .map_err(|error| format!("Le processus de mise à jour a échoué : {error}"));
    app.state::<Busy>().0.store(false, Ordering::Release);
    result?.map_err(LauncherError::from)
}

#[tauri::command]
async fn launch_game(app: tauri::AppHandle, client_dir: String) -> Result<(), LauncherError> {
    if app.state::<Busy>().0.swap(true, Ordering::AcqRel) {
        return Err(LauncherError::from(
            "Une opération est déjà en cours.".to_string(),
        ));
    }
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let root = PathBuf::from(client_dir);
        let http = http_client()?;
        let loaded = updater::fetch_manifest(&http, MANIFEST_URL, updater::public_key()?)?;
        if !updater::client_status_against_manifest(&root, &loaded)?.can_launch {
            return Err("Mise à jour obligatoire avant de jouer.".into());
        }
        let wow = updater::wow_path(&root)?;
        let mut child = Command::new(wow)
            .current_dir(root)
            .spawn()
            .map_err(|error| format!("Lancement de Wow.exe impossible : {error}"))?;
        let _ = worker_app.emit("game-started", ());
        let result = child
            .wait()
            .map(|_| ())
            .map_err(|error| format!("Suivi de Wow.exe impossible : {error}"));
        worker_app.state::<Busy>().0.store(false, Ordering::Release);
        let _ = worker_app.emit("game-exited", ());
        result
    })
    .await
    .map_err(|error| format!("Le processus du jeu a échoué : {error}"));
    app.state::<Busy>().0.store(false, Ordering::Release);
    result?.map_err(LauncherError::from)
}

fn main() {
    tauri::Builder::default()
        .manage(Busy(AtomicBool::new(false)))
        .manage(PendingLauncherUpdate::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            choose_client_dir,
            client_status,
            launcher_news,
            check_launcher_update,
            install_launcher_update,
            update_client,
            launch_game
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Rivals Beyond launcher");
}
