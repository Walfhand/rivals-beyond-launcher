# Rivals Beyond Launcher

Public, reproducible source for the Windows launcher of **Rivals Beyond**.

The launcher installs, repairs and starts the game client from signed manifests. It also updates itself from immutable, signed NSIS releases. This repository contains the launcher and its visual assets; it does not contain the game client, server, accounts or private signing keys.

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
