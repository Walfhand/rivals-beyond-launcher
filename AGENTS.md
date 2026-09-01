# AGENTS.md

## Repository authority

This public repository is the single production authority for the Rivals Beyond launcher: its Tauri
application, Rust updater, UI, packaging, public documentation and CI.

The private game repository remains authoritative for composing and signing the game-client manifest,
building `PATCH-Z.MPQ`, patching `Wow.exe` and selecting payload files. A payload-contract change must
be implemented on both sides: the private generator emits and validates it; this public updater accepts,
installs or retires it.

Never add manifest private keys, Tauri private keys, S3 credentials, ignored client files or private
release state here. Public verification keys are intentionally tracked.

## Current native client contract

The signed game manifest must contain `WarcraftXL.dll` and
`Extensions/RivalsBeyond/RivalsBeyond.dll` plus the pinned
`Extensions/UnitOutline/UnitOutline.dll` and its official rendering dependency
`Extensions/wxl-modern-m2/wxl-modern-m2.dll`. The updater removes the retired
`AwesomeWotlkLib.dll` only after a successful manifest stops listing it. It must never retain a second
runtime as a fallback.

WarcraftXL Hub is not part of the launcher. Install the pinned core and extensions directly
from the signed game manifest; do not add its community store, profiles or a second updater.

Run the updater tests after changing this contract:

```bash
cargo test --manifest-path launcher/src-tauri/Cargo.toml --no-default-features --lib
```
