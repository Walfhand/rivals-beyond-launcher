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

There is one game installation: common files plus optional `frFR` and `enUS` language packs.
On first use, the launcher follows the Windows user-interface language: French for any French
variant, English otherwise. Settings can override this choice; the override survives restarts.
The initial game language follows the launcher. An existing game's saved language and an explicit
game-language choice are preserved independently. Settings can download another available pack
without reinstalling common files. A missing selected pack blocks Play until installation finishes.

The active pack is downloaded first, followed by common files and any other retained/requested
packs. Updates and Repair maintain every previously installed pack. Only locales advertised by the
signed manifest are offered. Each installed locale gets the managed realm address; unrelated game
settings and saved key bindings are preserved.

The signed envelope remains schema 1. Its payload schema 2 adds `locales` and an optional `locale`
on language file entries; common files omit it. The aggregate file count and byte count cover the
entire signed payload. The updater validates every locale/path assignment and each complete native
archive chain, then computes transfer totals from the chosen files. Legacy schema 1 payloads remain
monolithic: all listed files are maintained together.

Release the compatible launcher **before** publishing a schema 2 game manifest; older launchers
reject the new payload. Until that manifest is published, a legacy manifest cannot provide optional
downloads.

The launcher reads its latest three articles directly from the website backend:
`https://api.rivalsbeyond.com/api/v1/news?locale=en&page=1&pageSize=3` (or `locale=fr`).
Article links open `https://rivalsbeyond.com/en/news/<slug>` or `/fr/news/<slug>`.
There is no bundled article list or separately published news feed. A validated API response is cached
per language for network outages; without a cache, a localized unavailable message appears.
News refreshes at startup, on language changes and during the ten-minute update check. Delayed
responses from a previous language cannot replace the selected language's articles.
The API is authenticated through HTTPS; game and launcher updates retain their signed manifests.

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
