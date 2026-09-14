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

`d3d9.dll` is the managed DXVK 2.7.1 payload. Before launching, the updater requires a Vulkan 1.3
physical device exposing every extension in `DXVK_REQUIRED_EXTENSIONS`; otherwise it moves the exact
manifest file to `d3d9.dll.dxvk`, and repair treats that parked copy as present. Once a compatible
device exists it restores the file. Never overwrite an unknown wrapper at either path.

Run the updater tests after changing this contract:

```bash
cargo test --manifest-path launcher/src-tauri/Cargo.toml --no-default-features --lib
```

## Launcher and game languages

One installation owns common files plus optional `frFR`/`enUS` packs. The signed envelope stays
schema 1; manifest payload schema 2 declares `locales` and tags each `Data/<locale>/` file with its
locale. Validate the complete signed payload before filtering. Download the active pack first,
retain installed packs during update/repair, and never report a selected missing pack ready.
Launcher language defaults to the Windows user UI language (all French variants => French,
otherwise English); saved manual choices win. Game language initially follows the launcher while
preserving an existing native selection or explicit game-language override. Never overwrite unrelated
`WTF/Config.wtf` preferences. UI strings live in `launcher/ui/locales.js`; progress uses stable phases.


## Website articles

`moba-web` owns published news. Read its existing public endpoint at
`https://api.rivalsbeyond.com/api/v1/news?locale=fr&page=1&pageSize=3` with `fr` or `en`
matching the launcher. Links use the same locale under `https://rivalsbeyond.com/<locale>/news`.
Do not restore a bundled article list, signed news JSON release or a second publishing pipeline.
Keep responses bounded, validate the requested locale and slugs, render text safely, and isolate
the offline cache by language. Game-client and launcher update signatures are unaffected.
