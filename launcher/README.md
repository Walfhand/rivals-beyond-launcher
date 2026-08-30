# Rivals Beyond launcher

Windows launcher for **Rivals Beyond**. It uses Tauri with a
plain HTML/CSS/JavaScript interface and a Rust updater.

The launcher can:

- select an empty folder or reuse the current 3.3.5a client;
- detect a new signed release automatically on startup, focus and game exit;
- verify a signed client manifest;
- resume interrupted downloads with HTTP Range;
- retry transient manifest and object failures without imposing a total deadline on large MPQs;
- verify every object with SHA-256 before replacing a file;
- block **Play** while an update is incomplete or a newer signed release exists;
- update without deleting screenshots, settings, addons or other unknown files;
- run a fast normal update or a full **Repair**;
- display a signed, remotely published news feed with a bundled/cache fallback;
- present actionable network, disk, permission, lock and security errors;
- update the launcher itself from a signed NSIS release before checking the game client;
- write `Data/frFR/realmlist.wtf`;
- launch `Wow.exe` without `-opengl`.

The launcher is x64; the WoW 3.3.5a client remains x86.

## Tests

The updater core is testable on Linux without the Tauri GUI dependencies:

```bash
cargo test \
  --manifest-path launcher/src-tauri/Cargo.toml \
  --no-default-features \
  --lib
python3 -m unittest \
  launcher/test_generate_manifest.py \
  launcher/test_publish_client.py \
  launcher/test_publish_launcher.py \
  launcher/test_ui.py \
  tools/client-patch/test_patch_wow_laa.py
```

## Client release

The ignored source client remains:

```text
docker/client/WINDOWS_World_of_Warcraft_335a/WINDOWS_World of Warcraft 335a/
```

Generate a new manifest. Increment `--sequence` for every published change:

```bash
python3 launcher/generate_manifest.py \
  "docker/client/WINDOWS_World_of_Warcraft_335a/WINDOWS_World of Warcraft 335a" \
  --version 2026.8.1.1 \
  --sequence 4
```

The inventory includes the base MPQs, Reforged/HD patches, the single custom
`Data/PATCH-Z.MPQ`, DXVK x86, the Awesome WotLK x86 DLL and the patched `Wow.exe`.
It excludes accounts, settings, caches, logs, crashes, loose developer files,
backups and `realmlist.wtf`.

Before generating it, run `make patch-exe`: manifest generation refuses a
client without Awesome WotLK, its pinned DLL, Large Address Aware, or one that
still carries the retired Ferrailleuse anti-dismount patch.

The Ed25519 release key is local and ignored:

```text
launcher/secrets/manifest-ed25519.pem
```

Never commit it. Back it up securely; the committed
`launcher/manifest-public-key.hex` must continue to match it.

Verify and sign without uploading:

```bash
python3 launcher/publish_client.py \
  "docker/client/WINDOWS_World_of_Warcraft_335a/WINDOWS_World of Warcraft 335a" \
  launcher/dist/client-manifest.json \
  launcher/secrets/manifest-ed25519.pem \
  --dry-run
```

Remove `--dry-run` to upload missing content-addressed objects, then publish the
signed manifest last. The script uses the local `moba-s3` AWS profile, endpoint
`https://nbg1.your-objectstorage.com`, bucket `moba-data`, and never embeds S3
credentials in the launcher.

The same command signs `launcher/news.json` with the client release key and
publishes it to `launcher/news/stable.json` after the client manifest. News is
plain structured text rendered with `textContent`; it cannot inject remote HTML.

The launcher reads:

```text
https://moba-data.nbg1.your-objectstorage.com/client/manifests/stable.json
https://moba-data.nbg1.your-objectstorage.com/launcher/news/stable.json
https://moba-data.nbg1.your-objectstorage.com/launcher/releases/stable.json
```

### Migration to the single custom patch

Distribute launcher `0.2.0` before publishing a manifest that no longer lists
the old custom archives. The new launcher remains compatible with the existing
manifest. With the next manifest it downloads and verifies `PATCH-Z.MPQ`, then
removes only:

```text
Data/patch-4.MPQ
Data/frFR/patch-frFR-4.MPQ
```

Do not publish the new manifest before users can install launcher `0.2.0`.

## Windows build

Run the **Build Windows launcher** workflow in GitHub Actions. Supply the
deployed realm hostname or IP; the workflow produces an NSIS installer artifact.
The realm field has no default to prevent an accidental public build pointing
to `127.0.0.1`; use that address only for a local test.

For a local Tauri build, install the platform prerequisites and Tauri CLI, then
run from `launcher/`:

```bash
MOBA_REALM_ADDRESS=127.0.0.1 cargo tauri dev
```

The interface reuses the existing MOBA login illustration and
**Rivals Beyond** logo. A code-signing certificate is
recommended before a broad release so Windows does not show an unknown
publisher warning.

### Automatic launcher updates

Launcher updates use Tauri's dedicated Minisign-compatible key, not the client
manifest Ed25519 key. The tracked public half is
`launcher/updater-public-key.pub`; the generated private half is ignored at
`launcher/secrets/launcher-updater.key`. Back up the private key outside this
checkout before the first release: losing it makes every installed launcher
impossible to update.

Configure these GitHub Actions secrets before building `0.3.0`:

```text
TAURI_SIGNING_PRIVATE_KEY          contents of launcher/secrets/launcher-updater.key
TAURI_SIGNING_PRIVATE_KEY_PASSWORD  empty for the currently generated key
MOBA_S3_ACCESS_KEY_ID
MOBA_S3_SECRET_ACCESS_KEY
MOBA_S3_REGION
```

Increment the matching version in `Cargo.toml` and `tauri.conf.json`, then run
the Windows workflow. Keep `publish` disabled for a test build. With `publish`
enabled, CI verifies the generated `.sig` against the tracked public key,
uploads the NSIS installer and signature under the immutable
`launcher/releases/<version>/` prefix, then publishes `stable.json` last.
`publish_launcher.py` refuses rollback, same-version replacement and an
artifact whose filename does not contain the requested version.

Keeping S3 credentials out of GitHub is also supported: leave `publish`
disabled, download the workflow's signed NSIS artifact and its `.sig`, then
publish from the trusted release machine that already owns the `moba-s3`
profile:

```bash
python3 launcher/publish_launcher.py \
  "/path/to/MOBA-Launcher_0.3.0_x64-setup.exe" \
  "/path/to/MOBA-Launcher_0.3.0_x64-setup.exe.sig" \
  --version 0.3.0 \
  --notes "Première version avec mise à jour automatique."
```

The installed launcher checks this feed before the game manifest. If a newer
SemVer exists it downloads, verifies and installs it in passive mode, then
restarts. Versions older than `0.3.0` do not contain this code and therefore
need one final manual `0.3.0` installation.
