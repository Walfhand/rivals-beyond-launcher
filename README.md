# Rivals Beyond Launcher

[![Build Windows launcher](https://github.com/Walfhand/rivals-beyond-launcher/actions/workflows/launcher-windows.yml/badge.svg)](https://github.com/Walfhand/rivals-beyond-launcher/actions/workflows/launcher-windows.yml)

**Rivals Beyond Launcher** is the open-source Windows launcher for
[Rivals Beyond](https://rivalsbeyond.com/fr), a community project that transforms WoW 3.3.5a into a
MOBA. It installs, repairs, updates and starts the game client from cryptographically signed manifests.

## What it does

- Preserves user-owned files while installing and repairing the managed game client.
- Verifies and applies signed launcher updates before game-client updates.
- Uses DXVK only when the system meets its Vulkan requirements, otherwise falling back to system D3D9.
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

## Security and privacy

- [Code signing policy](CODE_SIGNING_POLICY.md)
- [Privacy policy](PRIVACY.md)
- [Security policy](SECURITY.md)

The Tauri updater signature authenticates automatic updates.

## License

GNU General Public License v2.0. See [LICENSE](LICENSE).
