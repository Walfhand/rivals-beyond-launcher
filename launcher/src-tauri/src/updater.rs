use ed25519_dalek::{Signature, VerifyingKey};
use reqwest::{
    blocking::{Client, RequestBuilder, Response},
    header::{ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, RANGE},
    StatusCode, Url,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

const MAX_SIGNED_MANIFEST_SIZE: u64 = 16 * 1024 * 1024;
const MAX_MANIFEST_FILES: usize = 100_000;
const HTTP_MANIFEST_TIMEOUT: Duration = Duration::from_secs(60);
const HTTP_ATTEMPTS: u32 = 4;
const OBJECT_HOST: &str = "moba-data.nbg1.your-objectstorage.com";
const REQUIRED_FILES: &[&str] = &[
    "wow.exe",
    "d3d9.dll",
    "data/common.mpq",
    "data/patch-c.mpq",
    "data/patch-e.mpq",
    "data/patch-p.mpq",
    "data/patch-z.mpq",
    "data/frfr/locale-frfr.mpq",
];
const LEGACY_CUSTOM_PATCHES: &[&str] = &["Data/patch-4.MPQ", "Data/frFR/patch-frFR-4.MPQ"];
const PUBLIC_KEY_HEX: &str = include_str!("../../manifest-public-key.hex");

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub sequence: u64,
    pub client_version: String,
    pub object_base_url: String,
    pub file_count: usize,
    pub total_size: u64,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedDocument {
    schema_version: u32,
    payload: String,
    signature: String,
}

#[derive(Debug)]
pub struct LoadedManifest {
    pub manifest: Manifest,
    pub sha256: String,
}

#[derive(Clone, Serialize)]
pub struct Progress {
    pub message: String,
    pub items_done: u64,
    pub items_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

enum DownloadEvent {
    Bytes(u64),
    Retrying {
        bytes: u64,
        attempt: u32,
        delay: Duration,
    },
}

#[derive(Serialize)]
pub struct UpdateSummary {
    pub version: String,
    pub changed_files: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientState {
    InstallRequired,
    UpdateAvailable,
    Incomplete,
    Ready,
}

#[derive(Serialize)]
pub struct ClientStatus {
    pub state: ClientState,
    pub can_launch: bool,
    pub local_version: Option<String>,
    pub remote_version: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LocalState {
    schema_version: u32,
    sequence: u64,
    client_version: String,
    manifest_sha256: String,
}

impl Manifest {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("Version de manifeste non prise en charge.".into());
        }
        if self.sequence == 0 {
            return Err("La séquence du manifeste doit être positive.".into());
        }
        if self.client_version.is_empty()
            || self.client_version.len() > 64
            || !self.client_version.is_ascii()
        {
            return Err("Version client invalide dans le manifeste.".into());
        }
        if self.files.is_empty() || self.files.len() > MAX_MANIFEST_FILES {
            return Err("Nombre de fichiers invalide dans le manifeste.".into());
        }
        if self.file_count != self.files.len() {
            return Err("Le compteur de fichiers du manifeste est incohérent.".into());
        }
        validate_https_url(&self.object_base_url, true)?;

        let mut paths = HashSet::with_capacity(self.files.len());
        let mut total = 0u64;
        for entry in &self.files {
            validate_windows_path(&entry.path)?;
            if !paths.insert(entry.path.to_ascii_lowercase()) {
                return Err(format!(
                    "Chemin dupliqué sans tenir compte de la casse : {}",
                    entry.path
                ));
            }
            if entry.sha256.len() != 64
                || !entry
                    .sha256
                    .bytes()
                    .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
            {
                return Err(format!("SHA-256 invalide pour {}.", entry.path));
            }
            total = total
                .checked_add(entry.size)
                .ok_or_else(|| "La taille totale du manifeste déborde.".to_string())?;
        }
        if total != self.total_size {
            return Err("La taille totale du manifeste est incohérente.".into());
        }
        for required in REQUIRED_FILES {
            if !paths.contains(*required) {
                return Err(format!("Fichier client obligatoire absent : {required}"));
            }
        }
        Ok(())
    }
}

pub fn public_key() -> Result<[u8; 32], String> {
    decode_hex(PUBLIC_KEY_HEX.trim(), "clé publique")
}

pub fn fetch_manifest(
    client: &Client,
    manifest_url: &str,
    public_key: [u8; 32],
) -> Result<LoadedManifest, String> {
    validate_https_url(manifest_url, false)?;
    fetch_manifest_url(client, manifest_url, public_key)
}

fn fetch_manifest_url(
    client: &Client,
    manifest_url: &str,
    public_key: [u8; 32],
) -> Result<LoadedManifest, String> {
    let mut last_error = String::new();
    for attempt in 0..HTTP_ATTEMPTS {
        match fetch_manifest_attempt(client, manifest_url, public_key) {
            Ok(manifest) => return Ok(manifest),
            Err((retryable, error)) => {
                last_error = error;
                if !retryable || attempt + 1 == HTTP_ATTEMPTS {
                    break;
                }
                thread::sleep(retry_delay(attempt));
            }
        }
    }
    Err(last_error)
}

fn fetch_manifest_attempt(
    client: &Client,
    manifest_url: &str,
    public_key: [u8; 32],
) -> Result<LoadedManifest, (bool, String)> {
    let mut response = client
        .get(manifest_url)
        .header(ACCEPT_ENCODING, "identity")
        .timeout(HTTP_MANIFEST_TIMEOUT)
        .send()
        .map_err(|error| {
            (
                true,
                format!("Téléchargement du manifeste impossible : {error}"),
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err((
            is_retryable_status(status),
            format!("Manifeste indisponible ({status})."),
        ));
    }
    if response.content_length().unwrap_or(0) > MAX_SIGNED_MANIFEST_SIZE {
        return Err((false, "Le manifeste distant est trop volumineux.".into()));
    }
    let mut document = Vec::new();
    response
        .by_ref()
        .take(MAX_SIGNED_MANIFEST_SIZE + 1)
        .read_to_end(&mut document)
        .map_err(|error| (true, format!("Lecture du manifeste impossible : {error}")))?;
    if document.len() as u64 > MAX_SIGNED_MANIFEST_SIZE {
        return Err((false, "Le manifeste distant est trop volumineux.".into()));
    }
    load_signed_manifest(&document, public_key).map_err(|error| (false, error))
}

pub fn load_signed_manifest(
    document: &[u8],
    public_key: [u8; 32],
) -> Result<LoadedManifest, String> {
    if document.len() as u64 > MAX_SIGNED_MANIFEST_SIZE {
        return Err("Le manifeste signé est trop volumineux.".into());
    }
    let payload = verify_signed_document(document, public_key)?;
    let manifest: Manifest = serde_json::from_str(&payload)
        .map_err(|error| format!("Manifeste JSON invalide : {error}"))?;
    manifest.validate()?;
    Ok(LoadedManifest {
        manifest,
        sha256: sha256_bytes(payload.as_bytes()),
    })
}

pub(crate) fn verify_signed_document(
    document: &[u8],
    public_key: [u8; 32],
) -> Result<String, String> {
    let signed: SignedDocument = serde_json::from_slice(document)
        .map_err(|error| format!("Enveloppe signée invalide : {error}"))?;
    if signed.schema_version != 1 {
        return Err("Version d’enveloppe signée non prise en charge.".into());
    }
    let signature_bytes = decode_hex::<64>(&signed.signature, "signature")?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .map_err(|error| format!("Clé publique invalide : {error}"))?;
    verifying_key
        .verify_strict(
            signed.payload.as_bytes(),
            &Signature::from_bytes(&signature_bytes),
        )
        .map_err(|_| "Signature du document invalide.".to_string())?;
    Ok(signed.payload)
}

pub fn client_status(root: &Path) -> Result<ClientStatus, String> {
    validate_root(root)?;
    let wow = target_path(root, "Wow.exe", false)?;
    let has_client = regular_file_size(&wow)?.is_some();
    let incomplete = incomplete_update_exists(root)?;
    let can_launch = has_client && !incomplete;
    let local_version = read_state(root)
        .filter(|state| state.schema_version == 1)
        .map(|state| state.client_version);
    Ok(ClientStatus {
        state: if incomplete {
            ClientState::Incomplete
        } else if has_client {
            ClientState::Ready
        } else {
            ClientState::InstallRequired
        },
        can_launch,
        local_version,
        remote_version: None,
    })
}

pub fn client_status_against_manifest(
    root: &Path,
    loaded: &LoadedManifest,
) -> Result<ClientStatus, String> {
    let mut status = client_status(root)?;
    let matches = read_state(root).is_some_and(|state| {
        state.schema_version == 1
            && state.sequence == loaded.manifest.sequence
            && state.client_version == loaded.manifest.client_version
            && state.manifest_sha256 == loaded.sha256
    });
    status.remote_version = Some(loaded.manifest.client_version.clone());
    status.state = match status.state {
        ClientState::Incomplete => ClientState::Incomplete,
        ClientState::InstallRequired => ClientState::InstallRequired,
        _ if matches => ClientState::Ready,
        _ => ClientState::UpdateAvailable,
    };
    status.can_launch = status.state == ClientState::Ready;
    Ok(status)
}

pub fn wow_path(root: &Path) -> Result<PathBuf, String> {
    validate_root(root)?;
    if incomplete_update_exists(root)? {
        return Err("La mise à jour du client est incomplète. Relance-la avant de jouer.".into());
    }
    let wow = target_path(root, "Wow.exe", false)?;
    if regular_file_size(&wow)?.is_none() {
        return Err("Wow.exe est absent. Lance d’abord la mise à jour.".into());
    }
    Ok(wow)
}

pub fn update_client<F>(
    client: &Client,
    root: &Path,
    loaded: LoadedManifest,
    realm_address: &str,
    repair: bool,
    mut progress: F,
) -> Result<UpdateSummary, String>
where
    F: FnMut(Progress),
{
    validate_root(root)?;
    validate_realm_address(realm_address)?;
    let previous = read_state(root);
    if let Some(state) = &previous {
        if state.sequence > loaded.manifest.sequence {
            return Err("Mise à jour refusée : manifeste plus ancien que le client.".into());
        }
        if state.sequence == loaded.manifest.sequence && state.manifest_sha256 != loaded.sha256 {
            return Err("Mise à jour refusée : cette séquence désigne un autre manifeste.".into());
        }
    }
    let trust_sizes = !repair
        && previous
            .as_ref()
            .is_some_and(|state| state.manifest_sha256 == loaded.sha256);
    let changed = scan(root, &loaded.manifest, trust_sizes, &mut progress)?;
    let legacy_patches = legacy_custom_patches_to_remove(root, &loaded.manifest)?;
    let download_total = changed.iter().try_fold(0u64, |sum, entry| {
        sum.checked_add(entry.size)
            .ok_or_else(|| "La taille des téléchargements déborde.".to_string())
    })?;
    if !changed.is_empty() || !legacy_patches.is_empty() {
        write_incomplete_state(root, &loaded.sha256)?;
    }
    let mut completed_bytes = 0u64;
    for (index, entry) in changed.iter().enumerate() {
        let base = completed_bytes;
        download_and_replace(client, root, &loaded.manifest, entry, |event| {
            let (message, current) = match event {
                DownloadEvent::Bytes(current) => {
                    (format!("Téléchargement de {}", entry.path), current)
                }
                DownloadEvent::Retrying {
                    bytes,
                    attempt,
                    delay,
                } => (
                    format!(
                        "Connexion interrompue — nouvelle tentative {attempt}/{HTTP_ATTEMPTS} dans {} s",
                        delay.as_secs_f32()
                    ),
                    bytes,
                ),
            };
            progress(Progress {
                message,
                items_done: index as u64,
                items_total: changed.len() as u64,
                bytes_done: base.saturating_add(current),
                bytes_total: download_total,
            });
        })?;
        completed_bytes = completed_bytes.saturating_add(entry.size);
        progress(Progress {
            message: format!("Installation de {}", entry.path),
            items_done: (index + 1) as u64,
            items_total: changed.len() as u64,
            bytes_done: completed_bytes,
            bytes_total: download_total,
        });
    }

    remove_legacy_custom_patches(&legacy_patches)?;
    write_realmlist(root, realm_address)?;
    write_state(
        root,
        &LocalState {
            schema_version: 1,
            sequence: loaded.manifest.sequence,
            client_version: loaded.manifest.client_version.clone(),
            manifest_sha256: loaded.sha256,
        },
    )?;
    clear_incomplete_state(root)?;
    Ok(UpdateSummary {
        version: loaded.manifest.client_version,
        changed_files: changed.len() + legacy_patches.len(),
    })
}

fn remove_legacy_custom_patches(paths: &[PathBuf]) -> Result<(), String> {
    for path in paths {
        fs::remove_file(path).map_err(|error| {
            format!(
                "Suppression de l’ancien patch {} impossible : {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn legacy_custom_patches_to_remove(
    root: &Path,
    manifest: &Manifest,
) -> Result<Vec<PathBuf>, String> {
    let manifest_paths: HashSet<_> = manifest
        .files
        .iter()
        .map(|entry| entry.path.to_ascii_lowercase())
        .collect();
    let mut paths = Vec::new();
    for relative in LEGACY_CUSTOM_PATCHES {
        if manifest_paths.contains(&relative.to_ascii_lowercase()) {
            continue;
        }
        let path = target_path(root, relative, false)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !is_link_or_reparse(&metadata) => {
                paths.push(path)
            }
            Ok(_) => return Err(format!("Ancien patch local non sûr : {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Inspection de l’ancien patch {} impossible : {error}",
                    path.display()
                ))
            }
        }
    }
    Ok(paths)
}

fn scan<F>(
    root: &Path,
    manifest: &Manifest,
    trust_sizes: bool,
    progress: &mut F,
) -> Result<Vec<FileEntry>, String>
where
    F: FnMut(Progress),
{
    let mut changed = Vec::new();
    let mut completed_bytes = 0u64;
    for (index, entry) in manifest.files.iter().enumerate() {
        let target = target_path(root, &entry.path, false)?;
        let size = regular_file_size(&target)?;
        let matches = if size == Some(entry.size) {
            if trust_sizes {
                true
            } else {
                let base = completed_bytes;
                hash_file(&target, |read| {
                    progress(Progress {
                        message: format!("Vérification de {}", entry.path),
                        items_done: index as u64,
                        items_total: manifest.file_count as u64,
                        bytes_done: base.saturating_add(read),
                        bytes_total: manifest.total_size,
                    })
                })? == entry.sha256
            }
        } else {
            false
        };
        if !matches {
            changed.push(entry.clone());
        }
        completed_bytes = completed_bytes.saturating_add(entry.size);
        progress(Progress {
            message: format!("Vérification de {}", entry.path),
            items_done: (index + 1) as u64,
            items_total: manifest.file_count as u64,
            bytes_done: completed_bytes,
            bytes_total: manifest.total_size,
        });
    }
    Ok(changed)
}

fn download_and_replace<F>(
    client: &Client,
    root: &Path,
    manifest: &Manifest,
    entry: &FileEntry,
    mut progress: F,
) -> Result<(), String>
where
    F: FnMut(DownloadEvent),
{
    let target = target_path(root, &entry.path, true)?;
    let part = download_part_path(&target, &entry.sha256);
    if let Ok(metadata) = fs::symlink_metadata(&part) {
        if is_link_or_reparse(&metadata) || !metadata.is_file() {
            return Err(format!("Fichier partiel non sûr : {}", part.display()));
        }
        if metadata.len() > entry.size {
            File::create(&part)
                .map_err(|error| format!("Réinitialisation impossible : {error}"))?
                .sync_all()
                .map_err(|error| format!("Synchronisation impossible : {error}"))?;
        }
    }

    let url = Url::parse(&format!("{}{}", manifest.object_base_url, entry.sha256))
        .map_err(|error| format!("URL d’objet invalide : {error}"))?;
    let mut last_error = String::new();
    for attempt in 0..HTTP_ATTEMPTS {
        let current = regular_file_size(&part)?.unwrap_or(0);
        progress(DownloadEvent::Bytes(current));
        if current == entry.size {
            last_error.clear();
            break;
        }
        match download_attempt(client, url.clone(), &part, entry.size, |current| {
            progress(DownloadEvent::Bytes(current));
        }) {
            Ok(()) => {
                last_error.clear();
                break;
            }
            Err((retryable, error)) => {
                last_error = error;
                if !retryable || attempt + 1 == HTTP_ATTEMPTS {
                    break;
                }
                let delay = retry_delay(attempt);
                progress(DownloadEvent::Retrying {
                    bytes: regular_file_size(&part)?.unwrap_or(0),
                    attempt: attempt + 2,
                    delay,
                });
                thread::sleep(delay);
            }
        }
    }
    if !last_error.is_empty() {
        return Err(last_error);
    }
    if regular_file_size(&part)? != Some(entry.size) {
        return Err(format!("Téléchargement incomplet : {}", entry.path));
    }
    if hash_file(&part, |_| {})? != entry.sha256 {
        fs::remove_file(&part).map_err(|error| {
            format!(
                "Objet corrompu et suppression du partiel impossible ({}): {error}",
                entry.path
            )
        })?;
        return Err(format!("SHA-256 incorrect pour {}.", entry.path));
    }
    atomic_replace(&part, &target)?;
    Ok(())
}

fn download_attempt<F>(
    client: &Client,
    url: Url,
    part: &Path,
    expected_size: u64,
    mut progress: F,
) -> Result<(), (bool, String)>
where
    F: FnMut(u64),
{
    let mut start = regular_file_size(part)
        .map_err(|error| (false, error))?
        .unwrap_or(0);
    let mut response = object_request(client, url, start)
        .send()
        .map_err(|error| (true, format!("Connexion interrompue : {error}")))?;
    let status = response.status();
    if status == StatusCode::RANGE_NOT_SATISFIABLE && start == expected_size {
        return Ok(());
    }
    if status != StatusCode::OK && status != StatusCode::PARTIAL_CONTENT {
        return Err((
            is_retryable_status(status),
            format!("Téléchargement refusé par le serveur ({status})."),
        ));
    }

    let append = status == StatusCode::PARTIAL_CONTENT;
    if append {
        validate_content_range(&response, start, expected_size).map_err(|error| (false, error))?;
    } else {
        start = 0;
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > expected_size.saturating_sub(start))
    {
        return Err((false, "Le serveur annonce un objet trop volumineux.".into()));
    }

    let mut output = if append {
        OpenOptions::new().create(true).append(true).open(part)
    } else {
        File::create(part)
    }
    .map_err(|error| {
        (
            false,
            format!("Ouverture du fichier partiel impossible : {error}"),
        )
    })?;
    let mut written = start;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|error| (true, format!("Lecture réseau interrompue : {error}")))?;
        if count == 0 {
            break;
        }
        written = written
            .checked_add(count as u64)
            .ok_or_else(|| (false, "Taille téléchargée hors limites.".into()))?;
        if written > expected_size {
            return Err((false, "Le serveur a envoyé trop de données.".into()));
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| (false, format!("Écriture du client impossible : {error}")))?;
        progress(written);
    }
    output.sync_all().map_err(|error| {
        (
            false,
            format!("Synchronisation du client impossible : {error}"),
        )
    })?;
    if written != expected_size {
        return Err((
            true,
            format!("Objet incomplet ({written}/{expected_size} octets)."),
        ));
    }
    Ok(())
}

fn object_request(client: &Client, url: Url, start: u64) -> RequestBuilder {
    // Large MPQs may legitimately take hours. A per-request deadline turns an
    // active transfer into reqwest's misleading "body error" after 60 seconds.
    // ponytail: blocking reqwest has no inactivity-only timeout; migrate this
    // path to async read_timeout only if dead sockets remain open in production.
    let mut request = client.get(url).header(ACCEPT_ENCODING, "identity");
    if start > 0 {
        request = request.header(RANGE, format!("bytes={start}-"));
    }
    request
}

fn retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(500 * (1u64 << attempt.min(3)))
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn validate_content_range(
    response: &Response,
    expected_start: u64,
    expected_size: u64,
) -> Result<(), String> {
    let value = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "Réponse partielle sans Content-Range.".to_string())?;
    let value = value
        .strip_prefix("bytes ")
        .ok_or_else(|| "Content-Range invalide.".to_string())?;
    let (range, total) = value
        .split_once('/')
        .ok_or_else(|| "Content-Range invalide.".to_string())?;
    let (start, end) = range
        .split_once('-')
        .ok_or_else(|| "Content-Range invalide.".to_string())?;
    if start.parse::<u64>().ok() != Some(expected_start)
        || total.parse::<u64>().ok() != Some(expected_size)
        || end
            .parse::<u64>()
            .ok()
            .is_none_or(|end| end < expected_start || end >= expected_size)
    {
        return Err("Content-Range incohérent.".into());
    }
    Ok(())
}

fn validate_https_url(value: &str, require_trailing_slash: bool) -> Result<(), String> {
    let url = Url::parse(value).map_err(|error| format!("URL HTTPS invalide : {error}"))?;
    if url.scheme() != "https"
        || url.host_str() != Some(OBJECT_HOST)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (require_trailing_slash && !url.path().ends_with('/'))
    {
        return Err("Le manifeste doit utiliser l’URL HTTPS officielle.".into());
    }
    Ok(())
}

fn validate_windows_path(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 512
        || !value.is_ascii()
        || value.starts_with('/')
        || value.contains('\\')
    {
        return Err(format!("Chemin client non sûr : {value:?}"));
    }
    for part in value.split('/') {
        let base = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let reserved = matches!(base.as_str(), "con" | "prn" | "aux" | "nul")
            || (base.len() == 4
                && matches!(&base[..3], "com" | "lpt")
                && matches!(base.as_bytes()[3], b'1'..=b'9'));
        if part.is_empty()
            || matches!(part, "." | "..")
            || part.ends_with([' ', '.'])
            || reserved
            || part.bytes().any(|value| {
                value < 32 || matches!(value, b'<' | b'>' | b':' | b'"' | b'|' | b'?' | b'*')
            })
        {
            return Err(format!("Chemin client non sûr : {value:?}"));
        }
    }
    Ok(())
}

fn validate_realm_address(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 253
        || value.starts_with('.')
        || value.ends_with('.')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err("Adresse du realm invalide.".into());
    }
    Ok(())
}

fn validate_root(root: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("Dossier du client inaccessible : {error}"))?;
    if !metadata.is_dir() || is_link_or_reparse(&metadata) {
        return Err("Le dossier du client ne doit pas être un lien.".into());
    }
    Ok(())
}

fn target_path(root: &Path, relative: &str, create_parents: bool) -> Result<PathBuf, String> {
    validate_windows_path(relative)?;
    let mut target = root.to_path_buf();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        target.push(part);
        if parts.peek().is_some() {
            match fs::symlink_metadata(&target) {
                Ok(metadata) => {
                    if !metadata.is_dir() || is_link_or_reparse(&metadata) {
                        return Err(format!("Dossier client non sûr : {}", target.display()));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && create_parents => {
                    fs::create_dir(&target).map_err(|error| {
                        format!(
                            "Création du dossier {} impossible : {error}",
                            target.display()
                        )
                    })?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "Inspection de {} impossible : {error}",
                        target.display()
                    ))
                }
            }
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if is_link_or_reparse(&metadata) || !metadata.is_file() {
            return Err(format!("Fichier client non sûr : {}", target.display()));
        }
    }
    Ok(target)
}

fn regular_file_size(path: &Path) -> Result<Option<u64>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !is_link_or_reparse(&metadata) => {
            Ok(Some(metadata.len()))
        }
        Ok(_) => Err(format!("Chemin local non sûr : {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Inspection de {} impossible : {error}",
            path.display()
        )),
    }
}

fn is_link_or_reparse(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    false
}

fn hash_file<F>(path: &Path, mut progress: F) -> Result<String, String>
where
    F: FnMut(u64),
{
    let mut file = File::open(path)
        .map_err(|error| format!("Lecture de {} impossible : {error}", path.display()))?;
    let before = file
        .metadata()
        .map_err(|error| format!("Inspection de {} impossible : {error}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 8 * 1024 * 1024];
    let mut read = 0u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Lecture de {} impossible : {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        read = read.saturating_add(count as u64);
        progress(read);
    }
    let after = file
        .metadata()
        .map_err(|error| format!("Inspection de {} impossible : {error}", path.display()))?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || read != after.len()
    {
        return Err(format!(
            "{} a changé pendant sa vérification.",
            path.display()
        ));
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn decode_hex<const N: usize>(value: &str, name: &str) -> Result<[u8; N], String> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{name} hexadécimale invalide."));
    }
    let mut result = [0u8; N];
    for (index, output) in result.iter_mut().enumerate() {
        let offset = index * 2;
        *output = u8::from_str_radix(&value[offset..offset + 2], 16)
            .map_err(|_| format!("{name} hexadécimale invalide."))?;
    }
    Ok(result)
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn download_part_path(path: &Path, sha256: &str) -> PathBuf {
    sidecar_path(path, &format!(".moba.part.{sha256}"))
}

fn write_realmlist(root: &Path, realm_address: &str) -> Result<(), String> {
    let path = target_path(root, "Data/frFR/realmlist.wtf", true)?;
    atomic_write(
        &path,
        format!("set realmlist {realm_address}\r\n").as_bytes(),
    )
}

fn state_path(root: &Path) -> PathBuf {
    root.join(".moba-update").join("state.json")
}

fn incomplete_state_path(root: &Path) -> PathBuf {
    root.join(".moba-update").join("incomplete")
}

fn incomplete_update_exists(root: &Path) -> Result<bool, String> {
    Ok(regular_file_size(&incomplete_state_path(root))?.is_some())
}

fn read_state(root: &Path) -> Option<LocalState> {
    let path = state_path(root);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || is_link_or_reparse(&metadata) || metadata.len() > 64 * 1024 {
        return None;
    }
    serde_json::from_reader(File::open(path).ok()?).ok()
}

fn ensure_state_directory(root: &Path) -> Result<(), String> {
    let directory = root.join(".moba-update");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() && !is_link_or_reparse(&metadata) => {}
        Ok(_) => return Err("Le dossier d’état du launcher n’est pas sûr.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&directory)
            .map_err(|error| format!("Création de l’état du launcher impossible : {error}"))?,
        Err(error) => return Err(format!("Inspection de l’état impossible : {error}")),
    }
    Ok(())
}

fn write_incomplete_state(root: &Path, manifest_sha256: &str) -> Result<(), String> {
    ensure_state_directory(root)?;
    atomic_write(&incomplete_state_path(root), manifest_sha256.as_bytes())
}

fn clear_incomplete_state(root: &Path) -> Result<(), String> {
    match fs::remove_file(incomplete_state_path(root)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Suppression de l’état de mise à jour incomplète impossible : {error}"
        )),
    }
}

fn write_state(root: &Path, state: &LocalState) -> Result<(), String> {
    ensure_state_directory(root)?;
    let mut payload = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Sérialisation de l’état impossible : {error}"))?;
    payload.push(b'\n');
    atomic_write(&state_path(root), &payload)
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let part = sidecar_path(path, ".moba.part");
    if let Ok(metadata) = fs::symlink_metadata(&part) {
        if !metadata.is_file() || is_link_or_reparse(&metadata) {
            return Err(format!("Fichier temporaire non sûr : {}", part.display()));
        }
    }
    let mut file = File::create(&part)
        .map_err(|error| format!("Écriture de {} impossible : {error}", path.display()))?;
    file.write_all(content)
        .map_err(|error| format!("Écriture de {} impossible : {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("Synchronisation de {} impossible : {error}", path.display()))?;
    atomic_replace(&part, path)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(format!(
            "Remplacement atomique impossible : {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target)
        .map_err(|error| format!("Remplacement de {} impossible : {error}", target.display()))?;
    if let Some(parent) = target.parent() {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                format!(
                    "Synchronisation de {} impossible : {error}",
                    parent.display()
                )
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let unique = format!(
                "moba-launcher-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn valid_manifest() -> Manifest {
        let paths = [
            "Wow.exe",
            "d3d9.dll",
            "Data/common.MPQ",
            "Data/patch-C.mpq",
            "Data/patch-E.mpq",
            "Data/patch-P.mpq",
            "Data/PATCH-Z.MPQ",
            "Data/frFR/locale-frFR.MPQ",
        ];
        let files: Vec<_> = paths
            .into_iter()
            .map(|path| FileEntry {
                path: path.into(),
                size: 1,
                sha256: "00".repeat(32),
            })
            .collect();
        Manifest {
            schema_version: 1,
            sequence: 1,
            client_version: "test".into(),
            object_base_url: "https://moba-data.nbg1.your-objectstorage.com/client/objects/sha256/"
                .into(),
            file_count: files.len(),
            total_size: files.len() as u64,
            files,
        }
    }

    fn sign(manifest: &Manifest) -> (Vec<u8>, [u8; 32]) {
        let payload = serde_json::to_string(manifest).unwrap();
        let signing = SigningKey::from_bytes(&[7; 32]);
        let signature = signing.sign(payload.as_bytes());
        let document = serde_json::json!({
            "schema_version": 1,
            "payload": payload,
            "signature": signature.to_bytes().iter().map(|byte| format!("{byte:02x}")).collect::<String>()
        });
        (
            serde_json::to_vec(&document).unwrap(),
            signing.verifying_key().to_bytes(),
        )
    }

    #[test]
    fn signed_manifest_rejects_tampering_and_unsafe_paths() {
        let manifest = valid_manifest();
        let (document, key) = sign(&manifest);
        assert_eq!(
            load_signed_manifest(&document, key)
                .unwrap()
                .manifest
                .client_version,
            "test"
        );

        let mut tampered = document.clone();
        let position = tampered
            .windows(b"test".len())
            .position(|window| window == b"test")
            .unwrap();
        tampered[position] = b'X';
        assert!(load_signed_manifest(&tampered, key)
            .unwrap_err()
            .contains("Signature"));

        let mut unsafe_manifest = valid_manifest();
        unsafe_manifest.files[0].path = "Data/../Wow.exe".into();
        let (unsafe_document, key) = sign(&unsafe_manifest);
        assert!(load_signed_manifest(&unsafe_document, key)
            .unwrap_err()
            .contains("Chemin client non sûr"));
    }

    #[test]
    fn transient_manifest_failure_is_retried() {
        let (document, key) = sign(&valid_manifest());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut request = [0u8; 4096];
            let (mut socket, _) = listener.accept().unwrap();
            let _ = socket.read(&mut request).unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            drop(socket);

            let (mut socket, _) = listener.accept().unwrap();
            let _ = socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                document.len()
            )
            .unwrap();
            socket.write_all(&document).unwrap();
        });

        let client = Client::builder().timeout(None).build().unwrap();
        let loaded = fetch_manifest_url(&client, &format!("http://{address}/"), key).unwrap();
        server.join().unwrap();
        assert_eq!(loaded.manifest.client_version, "test");
    }

    #[test]
    fn large_object_downloads_have_no_total_request_deadline() {
        let client = Client::builder().timeout(None).build().unwrap();
        let request = object_request(&client, Url::parse("http://127.0.0.1/object").unwrap(), 0)
            .build()
            .unwrap();

        assert_eq!(request.timeout(), None);
    }

    #[test]
    fn launcher_status_names_install_update_incomplete_and_ready_states() {
        let directory = TestDir::new();
        let root = &directory.0;
        let loaded = LoadedManifest {
            manifest: valid_manifest(),
            sha256: "11".repeat(32),
        };

        let missing = client_status_against_manifest(root, &loaded).unwrap();
        assert_eq!(missing.state, ClientState::InstallRequired);
        assert_eq!(missing.remote_version.as_deref(), Some("test"));

        fs::write(root.join("Wow.exe"), b"existing client").unwrap();
        let outdated = client_status_against_manifest(root, &loaded).unwrap();
        assert_eq!(outdated.state, ClientState::UpdateAvailable);

        write_incomplete_state(root, &loaded.sha256).unwrap();
        let incomplete = client_status_against_manifest(root, &loaded).unwrap();
        assert_eq!(incomplete.state, ClientState::Incomplete);
        clear_incomplete_state(root).unwrap();

        write_state(
            root,
            &LocalState {
                schema_version: 1,
                sequence: loaded.manifest.sequence,
                client_version: loaded.manifest.client_version.clone(),
                manifest_sha256: loaded.sha256.clone(),
            },
        )
        .unwrap();
        let ready = client_status_against_manifest(root, &loaded).unwrap();
        assert_eq!(ready.state, ClientState::Ready);
        assert!(ready.can_launch);
    }

    #[test]
    fn legacy_custom_patches_are_removed_only_after_the_manifest_stops_listing_them() {
        let root = TestDir::new();
        let map_patch = root.0.join("Data/patch-4.MPQ");
        let locale_patch = root.0.join("Data/frFR/patch-frFR-4.MPQ");
        fs::create_dir_all(locale_patch.parent().unwrap()).unwrap();
        fs::write(&map_patch, b"map").unwrap();
        fs::write(&locale_patch, b"spells").unwrap();
        fs::write(root.0.join("Data/keep.MPQ"), b"keep").unwrap();

        let manifest = valid_manifest();
        let paths = legacy_custom_patches_to_remove(&root.0, &manifest).unwrap();
        assert_eq!(paths, vec![map_patch.clone(), locale_patch.clone()]);
        remove_legacy_custom_patches(&paths).unwrap();
        assert!(!map_patch.exists());
        assert!(!locale_patch.exists());
        assert_eq!(fs::read(root.0.join("Data/keep.MPQ")).unwrap(), b"keep");

        fs::write(&map_patch, b"map").unwrap();
        fs::write(&locale_patch, b"spells").unwrap();
        let mut old_manifest = manifest;
        for path in LEGACY_CUSTOM_PATCHES {
            old_manifest.files.push(FileEntry {
                path: (*path).into(),
                size: 1,
                sha256: "00".repeat(32),
            });
        }
        assert!(legacy_custom_patches_to_remove(&root.0, &old_manifest)
            .unwrap()
            .is_empty());
        assert!(map_patch.exists());
        assert!(locale_patch.exists());
        assert_eq!(fs::read(root.0.join("Data/keep.MPQ")).unwrap(), b"keep");
    }

    #[test]
    fn resumed_download_only_replaces_after_size_and_hash_match() {
        let directory = TestDir::new();
        let root = &directory.0;
        let target = root.join("Data").join("patch.MPQ");
        fs::create_dir(target.parent().unwrap()).unwrap();
        fs::write(&target, b"old file").unwrap();
        let content = b"complete immutable object";
        let entry = FileEntry {
            path: "Data/patch.MPQ".into(),
            size: content.len() as u64,
            sha256: sha256_bytes(content),
        };
        let part = download_part_path(&target, &entry.sha256);
        fs::write(&part, &content[..9]).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = content[9..].to_vec();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let count = socket.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            assert!(request.to_ascii_lowercase().contains("range: bytes=9-"));
            write!(
                socket,
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 9-{}/{}\r\nConnection: close\r\n\r\n",
                body.len(),
                content.len() - 1,
                content.len()
            )
            .unwrap();
            socket.write_all(&body).unwrap();
        });

        let manifest = Manifest {
            object_base_url: format!("http://{address}/"),
            files: vec![entry.clone()],
            file_count: 1,
            total_size: entry.size,
            ..valid_manifest()
        };
        let client = Client::builder().build().unwrap();
        download_and_replace(&client, root, &manifest, &entry, |_| {}).unwrap();
        server.join().unwrap();
        assert_eq!(fs::read(target).unwrap(), content);
    }

    #[test]
    fn interrupted_body_retries_from_the_downloaded_prefix() {
        let directory = TestDir::new();
        let root = &directory.0;
        let target = root.join("Data").join("patch.MPQ");
        fs::create_dir(target.parent().unwrap()).unwrap();
        let content = b"an interrupted body that resumes";
        let split = 11;
        let entry = FileEntry {
            path: "Data/patch.MPQ".into(),
            size: content.len() as u64,
            sha256: sha256_bytes(content),
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut request = [0u8; 4096];
            let (mut socket, _) = listener.accept().unwrap();
            let _ = socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                content.len()
            )
            .unwrap();
            socket.write_all(&content[..split]).unwrap();
            drop(socket);

            let (mut socket, _) = listener.accept().unwrap();
            let count = socket.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..count])
                .to_ascii_lowercase()
                .contains(&format!("range: bytes={split}-")));
            write!(
                socket,
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                content.len() - split,
                split,
                content.len() - 1,
                content.len()
            )
            .unwrap();
            socket.write_all(&content[split..]).unwrap();
        });

        let manifest = Manifest {
            object_base_url: format!("http://{address}/"),
            files: vec![entry.clone()],
            file_count: 1,
            total_size: entry.size,
            ..valid_manifest()
        };
        let client = Client::builder().timeout(None).build().unwrap();
        download_and_replace(&client, root, &manifest, &entry, |_| {}).unwrap();
        server.join().unwrap();
        assert_eq!(fs::read(target).unwrap(), content);
    }

    #[test]
    fn partial_download_is_reused_only_for_the_same_object() {
        let directory = TestDir::new();
        let root = &directory.0;
        let target = root.join("Data").join("patch.MPQ");
        fs::create_dir(target.parent().unwrap()).unwrap();
        let content = b"new immutable object";
        let entry = FileEntry {
            path: "Data/patch.MPQ".into(),
            size: content.len() as u64,
            sha256: sha256_bytes(content),
        };
        let old_sha = sha256_bytes(b"old immutable object");
        fs::write(download_part_path(&target, &old_sha), b"old prefix").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let count = socket.read(&mut request).unwrap();
            assert!(!String::from_utf8_lossy(&request[..count])
                .to_ascii_lowercase()
                .contains("range:"));
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                content.len()
            )
            .unwrap();
            socket.write_all(content).unwrap();
        });

        let manifest = Manifest {
            object_base_url: format!("http://{address}/"),
            files: vec![entry.clone()],
            file_count: 1,
            total_size: entry.size,
            ..valid_manifest()
        };
        let client = Client::builder().build().unwrap();
        download_and_replace(&client, root, &manifest, &entry, |_| {}).unwrap();
        server.join().unwrap();
        assert_eq!(fs::read(target).unwrap(), content);
    }

    #[test]
    fn failed_partial_install_blocks_launch_until_the_update_resumes() {
        let directory = TestDir::new();
        let root = &directory.0;
        fs::write(root.join("Wow.exe"), b"old wow").unwrap();
        let wow = b"new wow";
        let dll = b"new dll";
        let files = vec![
            FileEntry {
                path: "Wow.exe".into(),
                size: wow.len() as u64,
                sha256: sha256_bytes(wow),
            },
            FileEntry {
                path: "d3d9.dll".into(),
                size: dll.len() as u64,
                sha256: sha256_bytes(dll),
            },
        ];

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _count = socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                wow.len()
            )
            .unwrap();
            socket.write_all(wow).unwrap();

            let (mut socket, _) = listener.accept().unwrap();
            let _count = socket.read(&mut request).unwrap();
            socket
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        let manifest = Manifest {
            sequence: 2,
            client_version: "next".into(),
            object_base_url: format!("http://{address}/"),
            file_count: files.len(),
            total_size: files.iter().map(|entry| entry.size).sum(),
            files: files.clone(),
            ..valid_manifest()
        };
        let client = Client::builder().build().unwrap();
        assert!(update_client(
            &client,
            root,
            LoadedManifest {
                manifest: manifest.clone(),
                sha256: "11".repeat(32),
            },
            "127.0.0.1",
            false,
            |_| {},
        )
        .is_err());
        server.join().unwrap();
        assert_eq!(fs::read(root.join("Wow.exe")).unwrap(), wow);
        assert!(!client_status(root).unwrap().can_launch);
        assert!(wow_path(root).unwrap_err().contains("incomplète"));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _count = socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                dll.len()
            )
            .unwrap();
            socket.write_all(dll).unwrap();
        });
        let manifest = Manifest {
            object_base_url: format!("http://{address}/"),
            ..manifest
        };
        update_client(
            &client,
            root,
            LoadedManifest {
                manifest: manifest.clone(),
                sha256: "11".repeat(32),
            },
            "127.0.0.1",
            false,
            |_| {},
        )
        .unwrap();
        server.join().unwrap();
        assert!(client_status(root).unwrap().can_launch);
        assert!(wow_path(root).is_ok());
        let mut loaded = LoadedManifest {
            manifest,
            sha256: "11".repeat(32),
        };
        assert!(
            client_status_against_manifest(root, &loaded)
                .unwrap()
                .can_launch
        );
        loaded.sha256 = "22".repeat(32);
        assert!(
            !client_status_against_manifest(root, &loaded)
                .unwrap()
                .can_launch
        );
    }

    #[test]
    fn corrupt_download_never_replaces_an_existing_file() {
        let directory = TestDir::new();
        let root = &directory.0;
        let target = root.join("Data").join("patch.MPQ");
        fs::create_dir(target.parent().unwrap()).unwrap();
        fs::write(&target, b"known-good-old-file").unwrap();
        let expected = b"expected";
        let entry = FileEntry {
            path: "Data/patch.MPQ".into(),
            size: expected.len() as u64,
            sha256: sha256_bytes(expected),
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _count = socket.read(&mut request).unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nbad-data",
                )
                .unwrap();
        });

        let manifest = Manifest {
            object_base_url: format!("http://{address}/"),
            files: vec![entry.clone()],
            file_count: 1,
            total_size: entry.size,
            ..valid_manifest()
        };
        let client = Client::builder().build().unwrap();
        assert!(
            download_and_replace(&client, root, &manifest, &entry, |_| {})
                .unwrap_err()
                .contains("SHA-256")
        );
        server.join().unwrap();
        assert_eq!(fs::read(target).unwrap(), b"known-good-old-file");
    }
}
