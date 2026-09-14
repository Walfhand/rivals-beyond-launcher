# Rivals Beyond Launcher

[![Build Windows launcher](https://github.com/Walfhand/rivals-beyond-launcher/actions/workflows/launcher-windows.yml/badge.svg)](https://github.com/Walfhand/rivals-beyond-launcher/actions/workflows/launcher-windows.yml)

**Rivals Beyond Launcher** is the open-source Windows launcher for
[Rivals Beyond](https://rivalsbeyond.com/fr), a community project that transforms WoW 3.3.5a into a
MOBA. It installs, repairs, updates and starts the game client from cryptographically signed manifests.

## What it does

- Preserves user-owned files while installing and repairing the managed game client.
- Verifies and applies signed launcher updates before game-client updates.
- Includes the full Microsoft WebView2 installer and installs it silently when missing, without
  a separate dependency download during setup.
- Uses DXVK only when the system meets its Vulkan requirements, otherwise falling back to system D3D9.
- Defaults the in-game tutorial to off before launch when `WTF/Config.wtf` has no `showTutorials`
  setting, preserving existing gameplay and video preferences.
- Initializes a missing `gxResolution` from the primary monitor's physical pixel dimensions.
  Saved resolutions stay unchanged; if monitor detection fails, the game chooses its own default.
- Disables the native startup hardware preset pass (`hwDetect=0`) when a usable resolution is
  configured, so it does not replace the prepared mode on a fresh first launch.
- Skips the original and expansion intro movies and the legacy WoW notice screens through local
  startup CVars. This does not submit or store account-side acceptance of any service's terms.
- Links directly to the official account registration and Rivals Beyond news.

This repository contains the launcher source, interface, packaging and CI configuration. It does not
contain the game client, server, accounts or private signing keys.

- [Official website](https://rivalsbeyond.com/fr)
- [Create an account](https://rivalsbeyond.com/register)
- [Download the latest release](https://github.com/Walfhand/rivals-beyond-launcher/releases/latest)

## Build and test

The updater core is testable without the Windows desktop runtime:

```bash
cargo test --manifest-path launcher/src-tauri/Cargo.toml --no-default-features --lib
python3 -m unittest \
  launcher/test_publish_launcher.py \
  launcher/test_ui.py
```

Windows NSIS installers are built from this repository by [GitHub Actions](.github/workflows/launcher-windows.yml). The workflow tests the Rust updater and release tooling, builds the installer, verifies its Tauri updater signature and uploads the resulting artifacts.

Builds default to the game realm `moba.rivalsbeyond.com`. The workflow's `realm_address` input
sets `MOBA_REALM_ADDRESS` at compile time; local builds can override the same environment variable.

## Client languages

The signed game payload supports French and English language archives. The launcher writes the
managed realm address for both installed locales (`frFR` and `enUS`). Select the language in WoW's
native Interface → Languages panel, then restart the game. Existing keyboard bindings are preserved.

## Automatic client diagnostics

Automatic Sentry delivery is enabled by default; the settings checkbox remembers a player's opt-out.
`launcher/src-tauri/src/diagnostics.rs` relays the game's bounded `Logs/RivalsDiagnostics*.jsonl`
journals, captures WarcraftXL warnings/errors and records successful game starts, duration and exit
status. Network delivery runs on a separate worker. The game build must include the matching native
collector and FrameXML from the private game repository; `make client` alone does not start this relay.

In Sentry, filter `kind:game_started environment:production` for launch counts, `kind:client_crash`
for abnormal exits, `component:world_entry` for minimap/HUD checkpoints or `component:champion_gallery`
for window layering. Use `client_version` and the ephemeral `session` tag to correlate events. These
are Sentry events/issues rather than an unbounded stream of raw logs; quotas and local rotation mean
counts can be incomplete. No unique-player count is collected.

Resolution diagnostics include the detected monitor size, the preserved/written resolution and the
selection reason (`saved`, `monitor`, `client_default` or `saved_invalid` when the stored value cannot
be parsed). Compare `resolution_configured` on the launch event with the game's `resolution` tag on
the first world-entry checkpoint, using the same `session` tag. Other settings/account values are
not included in this report.

`hw_detect_disabled` records whether startup preparation disabled the native hardware preset pass.
To reproduce a fresh launch, close WoW, back up/remove `WTF/Config.wtf`, then start the game through
the updated launcher. The selected resolution and the startup flags are written together before
`Wow.exe` is spawned. Existing video settings are retained; the local legacy boot flags are enforced
even if an old config still requests the movies/notices. Files retain their original encoding and
unrelated contents, and repeating preparation does not append duplicate overrides.

Tests: `cargo test --manifest-path launcher/src-tauri/Cargo.toml --no-default-features --lib`.
An explicit smoke test sends one synthetic development event to the configured project:
`cargo run --manifest-path launcher/src-tauri/Cargo.toml --no-default-features --example diagnostics_smoke -- --send`.

## Security and privacy

- [Code signing policy](CODE_SIGNING_POLICY.md)
- [Privacy policy](PRIVACY.md)
- [Security policy](SECURITY.md)

The Tauri updater signature authenticates automatic updates.

## License

GNU General Public License v2.0. See [LICENSE](LICENSE).
