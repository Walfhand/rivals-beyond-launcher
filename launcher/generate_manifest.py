#!/usr/bin/env python3
"""Build a deterministic manifest for the distributable WoW client."""

import argparse
import hashlib
import json
import os
import stat
import sys
import tempfile
from collections.abc import Callable
from pathlib import Path
from urllib.parse import urlsplit


CLIENT_PATCH_TOOLS = Path(__file__).resolve().parents[1] / "tools/client-patch"
sys.path.insert(0, str(CLIENT_PATCH_TOOLS))
import patch_wow_laa


ROOT_FILES = {
    name.casefold()
    for name in (
        "Battle.net.dll",
        "AwesomeWotlkLib.dll",
        "DivxDecoder.dll",
        "Wow.exe",
        "WowError.exe",
        "d3d9.dll",
        "dbghelp.dll",
        "ijl15.dll",
        "msvcr80.dll",
        "unicows.dll",
    )
}
EXCLUDED_ROOT_DIRECTORIES = {
    "cache",
    "errors",
    "logs",
    "screenshots",
    "updates",
    "wtf",
    ".moba-update",
}
EXCLUDED_SUFFIXES = (".orig", ".pre-laa", ".part", ".tmp")
REQUIRED_FILES = (
    "Wow.exe",
    "AwesomeWotlkLib.dll",
    "d3d9.dll",
    "Data/common.MPQ",
    "Data/patch-C.mpq",
    "Data/patch-E.mpq",
    "Data/patch-P.mpq",
    "Data/PATCH-Z.MPQ",
    "Data/frFR/locale-frFR.MPQ",
)
LEGACY_CUSTOM_PATCHES = {
    "data/patch-4.mpq",
    "data/frfr/patch-frfr-4.mpq",
}
WINDOWS_RESERVED_NAMES = {
    "con",
    "prn",
    "aux",
    "nul",
    *(f"com{number}" for number in range(1, 10)),
    *(f"lpt{number}" for number in range(1, 10)),
}
DEFAULT_OBJECTS_URL = (
    "https://moba-data.nbg1.your-objectstorage.com/client/objects/sha256/"
)
OBJECTS_NETLOC = urlsplit(DEFAULT_OBJECTS_URL).netloc
DEFAULT_OUTPUT = Path(__file__).resolve().parent / "dist/client-manifest.json"
INVALID_WINDOWS_CHARACTERS = set('<>"|?*')


def _included(relative: Path) -> bool:
    parts = relative.parts
    lower_path = relative.as_posix().casefold()
    if not parts or parts[0].casefold() in EXCLUDED_ROOT_DIRECTORIES:
        return False
    if lower_path.endswith(EXCLUDED_SUFFIXES):
        return False

    top = parts[0].casefold()
    if top == "data":
        suffix = relative.suffix.casefold()
        if len(parts) == 2:
            return suffix == ".mpq"
        if len(parts) < 3 or parts[1].casefold() != "frfr":
            return False
        if len(parts) == 3:
            return suffix == ".mpq"
        if parts[2].casefold() == "custom":
            return suffix == ".mpq"
        return (
            lower_path == "data/frfr/wtf/defaultbindings.wtf"
            or (
                len(parts) == 5
                and parts[2].casefold() == "interface"
                and parts[3].casefold() == "cinematics"
                and suffix == ".avi"
            )
        )
    if top == "interface":
        return (
            len(parts) == 4
            and parts[1].casefold() == "addons"
            and parts[2].casefold().startswith("blizzard_")
            and relative.suffix.casefold() == ".pub"
        ) or (
            len(parts) >= 4
            and parts[1].casefold() == "addons"
            and parts[2].casefold() == "mobalevel1pvp"
        )
    return len(parts) == 1 and parts[0].casefold() in ROOT_FILES


def _validate_windows_path(relative: Path) -> str:
    value = relative.as_posix()
    if not value.isascii() or len(value) > 512:
        raise ValueError(f"unsupported Windows path: {value!r}")
    for part in relative.parts:
        if (
            not part
            or part in {".", ".."}
            or any(ord(character) < 32 for character in part)
            or any(character in INVALID_WINDOWS_CHARACTERS for character in part)
            or "\\" in part
            or ":" in part
            or part.endswith((" ", "."))
            or part.split(".", 1)[0].casefold() in WINDOWS_RESERVED_NAMES
        ):
            raise ValueError(f"unsafe Windows path: {value!r}")
    return value


def _normalize_objects_url(value: str) -> str:
    parsed = urlsplit(value)
    if (
        parsed.scheme != "https"
        or parsed.netloc.casefold() != OBJECTS_NETLOC
        or parsed.query
        or parsed.fragment
        or not parsed.path.endswith("/")
    ):
        raise ValueError(
            "object base URL must use the official HTTPS origin and end with '/'"
        )
    return value


def _validate_d3d9(path: Path) -> None:
    try:
        patch_wow_laa.pe_offsets(path.read_bytes())
    except ValueError as error:
        # pe_offsets performs every PE32/x86 check before its WoW-only marker check.
        if str(error) == "expected WoW 3.3.5a build 12340":
            return
        raise ValueError("d3d9.dll must be a PE32 x86/i386 DLL") from error


def _validate_wow(path: Path) -> None:
    data = path.read_bytes()
    characteristics_offset, _ = patch_wow_laa.pe_offsets(data)
    characteristics = int.from_bytes(
        data[characteristics_offset:characteristics_offset + 2], "little"
    )
    if not characteristics & patch_wow_laa.LARGE_ADDRESS_AWARE:
        raise ValueError(
            "Wow.exe must be Large Address Aware: run 'make patch-exe' before publishing"
        )

    start = patch_wow_laa.MOUNT_CANCEL_OFFSET
    end = start + len(patch_wow_laa.MOUNT_CANCEL_ORIGINAL)
    present = data[start:end]
    if present == patch_wow_laa.MOUNT_CANCEL_PATCHED:
        raise ValueError(
            "Wow.exe still contains the retired Ferrailleuse mount hack: "
            "run 'make patch-exe' before publishing"
        )
    if present != patch_wow_laa.MOUNT_CANCEL_ORIGINAL:
        raise ValueError("Wow.exe does not contain the expected stock dismount routine")
    if patch_wow_laa.awesome_patch_state(data) != "patched":
        raise ValueError("Wow.exe must contain the Awesome WotLK loader patch: run 'make patch-exe'")


def _client_files(root: Path) -> list[tuple[str, Path]]:
    if root.is_symlink() or not root.is_dir():
        raise ValueError(f"client directory not found: {root}")
    root = root.resolve()

    for required in REQUIRED_FILES:
        path = root / required
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"missing required client file: {required}")
    _validate_wow(root / "Wow.exe")
    _validate_d3d9(root / "d3d9.dll")
    patch_wow_laa.validate_awesome_dll((root / "AwesomeWotlkLib.dll").read_bytes())

    files = []
    seen = {}
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"refusing client symlink: {path.relative_to(root)}")
        mode = path.lstat().st_mode
        if stat.S_ISDIR(mode):
            continue
        if not stat.S_ISREG(mode):
            raise ValueError(f"refusing special client file: {path.relative_to(root)}")

        relative = path.relative_to(root)
        if relative.as_posix().casefold() in LEGACY_CUSTOM_PATCHES:
            raise ValueError(f"legacy custom patch must be removed: {relative}")
        if not _included(relative):
            continue
        manifest_path = _validate_windows_path(relative)
        folded = manifest_path.casefold()
        if folded in seen:
            raise ValueError(
                "case-insensitive path collision: "
                f"{seen[folded]!r} and {manifest_path!r}"
            )
        seen[folded] = manifest_path
        files.append((manifest_path, path))

    return sorted(files, key=lambda entry: entry[0].casefold())


def _hash_file(path: Path) -> tuple[int, str]:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        before = os.fstat(source.fileno())
        for chunk in iter(lambda: source.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
        after = os.fstat(source.fileno())
    current = path.stat()
    expected = (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
    if expected != (
        after.st_dev,
        after.st_ino,
        after.st_size,
        after.st_mtime_ns,
    ) or expected != (
        current.st_dev,
        current.st_ino,
        current.st_size,
        current.st_mtime_ns,
    ):
        raise OSError(f"file changed while hashing: {path}")
    return current.st_size, digest.hexdigest()


def build_manifest(
    root: Path,
    version: str,
    objects_url: str,
    sequence: int = 1,
    progress: Callable[[int, int, str], None] | None = None,
) -> dict:
    root = Path(root)
    if not version or not version.isascii() or len(version) > 64:
        raise ValueError("client version must be 1-64 ASCII characters")
    if sequence < 1:
        raise ValueError("manifest sequence must be positive")

    objects_url = _normalize_objects_url(objects_url)
    source_files = _client_files(root)
    entries = []
    total = len(source_files)
    for index, (relative, path) in enumerate(source_files, 1):
        if progress:
            progress(index, total, relative)
        size, sha256 = _hash_file(path)
        entries.append({"path": relative, "size": size, "sha256": sha256})

    return {
        "schema_version": 1,
        "sequence": sequence,
        "client_version": version,
        "object_base_url": objects_url,
        "file_count": len(entries),
        "total_size": sum(entry["size"] for entry in entries),
        "files": entries,
    }


def write_manifest(output: Path, manifest: dict) -> None:
    output = Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    payload = (
        json.dumps(manifest, ensure_ascii=True, indent=2, sort_keys=True) + "\n"
    ).encode()
    temporary_name = None
    try:
        with tempfile.NamedTemporaryFile(
            prefix=output.name + ".",
            suffix=".tmp",
            dir=output.parent,
            delete=False,
        ) as temporary:
            temporary_name = temporary.name
            temporary.write(payload)
            temporary.flush()
            os.fsync(temporary.fileno())
        os.replace(temporary_name, output)
        temporary_name = None
    finally:
        if temporary_name:
            Path(temporary_name).unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build the signed-updater input manifest for the MOBA client."
    )
    parser.add_argument("client_dir", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sequence", type=int, default=1)
    parser.add_argument("--objects-url", default=DEFAULT_OBJECTS_URL)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()

    client_dir = args.client_dir.resolve()
    output = args.output.resolve()
    if output == client_dir or client_dir in output.parents:
        print("error: manifest output must be outside the client", file=sys.stderr)
        return 1

    def show_progress(index: int, total: int, path: str) -> None:
        print(f"[{index}/{total}] {path}", file=sys.stderr, flush=True)

    try:
        manifest = build_manifest(
            client_dir,
            version=args.version,
            objects_url=args.objects_url,
            sequence=args.sequence,
            progress=show_progress,
        )
        write_manifest(output, manifest)
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    gib = manifest["total_size"] / (1024 ** 3)
    print(f"Manifest: {output}")
    print(f"Files: {manifest['file_count']}")
    print(f"Size: {manifest['total_size']} bytes ({gib:.2f} GiB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
