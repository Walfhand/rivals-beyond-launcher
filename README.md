# Rivals Beyond Launcher

[![Build Windows launcher](https://github.com/Walfhand/rivals-beyond-launcher/actions/workflows/launcher-windows.yml/badge.svg)](https://github.com/Walfhand/rivals-beyond-launcher/actions/workflows/launcher-windows.yml)

Public, reproducible source for the Windows launcher of **Rivals Beyond**.

The launcher installs, repairs and starts the game client from signed manifests. It also updates itself from immutable, signed NSIS releases. This repository contains the launcher and its visual assets; it does not contain the game client, server, accounts or private signing keys.

The header opens the official account form at `https://rivalsbeyond.com/register`. News cards mirror
the latest published French articles and open their canonical pages on `rivalsbeyond.com` in the
system browser; Tauri's opener permission is scoped to that origin only.

The game-client manifest is composed and signed outside this public repository. The updater currently
requires `WarcraftXL.dll`, `Extensions/RivalsBeyond/RivalsBeyond.dll` and the pinned
`Extensions/UnitOutline/UnitOutline.dll` plus `Extensions/wxl-modern-m2/wxl-modern-m2.dll`, which
owns the M2 draw event Unit Outline consumes; after installing that payload
it removes the retired `AwesomeWotlkLib.dll`. No private signing material is needed to build or test
this behavior.

Release launcher `0.3.2` before publishing the first WarcraftXL game manifest. Launcher self-update
runs before the game-client update, which guarantees the retired DLL cleanup code is present first.

## Build and test

The updater core is testable without the Windows desktop runtime:

```bash
cargo test --manifest-path launcher/src-tauri/Cargo.toml --no-default-features --lib
python3 -m unittest \
  launcher/test_publish_launcher.py \
  launcher/test_ui.py
```

Windows NSIS installers are built from this repository by [GitHub Actions](.github/workflows/launcher-windows.yml). The workflow tests the Rust updater and release tooling, builds the installer, verifies its Tauri updater signature and uploads the resulting artifacts.

## Security and privacy

- [Code signing policy](CODE_SIGNING_POLICY.md)
- [Privacy policy](PRIVACY.md)
- [Security policy](SECURITY.md)

The Tauri updater signature authenticates automatic updates. Authenticode signing through SignPath Foundation is pending project approval.

## License

GNU General Public License v2.0. See [LICENSE](LICENSE).
