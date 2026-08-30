#!/usr/bin/env python3
"""Apply the repository's validated WoW 3.3.5a executable patches."""

import argparse
import hashlib
import os
import shutil
import struct
import sys
import tempfile
from pathlib import Path


BUILD_MARKER = b"World of WarCraft (build 12340)"
LARGE_ADDRESS_AWARE = 0x0020

# --- One-time cleanup for clients that received the retired Ferrailleuse mount hack -------------
#
# Older project builds replaced the first byte of the client's cancel-mount routine with `ret`. The mech
# is now display 60011, a regular morph, so blocking every client-side dismount is both unnecessary and
# harmful to real mounts such as Élan draconique. Keep this exact reversal while those clients exist.
MOUNT_CANCEL_OFFSET = 0x3406E0
MOUNT_CANCEL_ORIGINAL = bytes((0x55, 0x8B, 0xEC, 0x83, 0xEC, 0x18))   # push ebp; mov ebp,esp; sub esp,0x18
MOUNT_CANCEL_PATCHED = bytes((0xC3,)) + MOUNT_CANCEL_ORIGINAL[1:]     # ret; (rest becomes unreachable)
ROOT = Path(__file__).resolve().parents[2]
AWESOME_DLL_NAME = "AwesomeWotlkLib.dll"
AWESOME_DLL = ROOT / "client-patches/awesome-wotlk" / AWESOME_DLL_NAME
AWESOME_DLL_SHA256 = "370e846e7e9bcef5d5e1b47324819a56ded1f1300039f58574b41924705fd40c"
AWESOME_DLL_PREVIOUS_SHA256 = {
    "4f706e287b1a5e9b87983f16980b48642f21758d85942a8d45d04d645716b6e8",
    # Upstream 0.1.4 DLL shipped by launcher 2026.8.24.1 and pinned in parent commit 813a26b1.
    "73496e61c9fec6fe5e948dd1871b17b7e06877ae9f3e318440ef7abde785650b",
}
AWESOME_DLL_API_MARKERS = (
    b"C_MobaPing\0",
    b"GetCursorWorldPosition\0",
    b"ProjectWorldPosition\0",
    b"ProjectUnit\0",
)

# Awesome WotLK 0.1.4 patches these three functions in the stock build 12340. Raw offsets are used,
# like MOUNT_CANCEL_OFFSET above, so every write is guarded by the exact stock preimage.
AWESOME_PATCHES = (
    (
        0x0DC0F0,
        bytes.fromhex("558bec568b75"),
        bytes.fromhex("b800000000c3"),
    ),
    (
        0x0E50B0,
        bytes.fromhex(
            "558bec5633f639356cb4b6000f85db010000393568b4b6000f85cf01000033c0"
            "b968b4b6008701566a5468f8659f006a18e85a8828006860659f00a380b4b600"
            "e81bbef7"
        ),
        bytes.fromhex(
            "b801000000a374b4b60068e05c4e00e81c68380083c404558bece8a110f2ffe9"
            "045bf2ffcccccccccccccccccccccccc417765736f6d65576f746c6b4c69622e64"
            "6c6c00"
        ),
    ),
    (
        0x00ABD0,
        bytes.fromhex("558bece898b5ffff"),
        bytes.fromhex("e9dba40d00909090"),
    ),
)
DEFAULT_WOW = (
    ROOT
    / "docker/client/WINDOWS_World_of_Warcraft_335a"
    / "WINDOWS_World of Warcraft 335a/Wow.exe"
)


def pe_offsets(data: bytes) -> tuple[int, int]:
    if len(data) < 0x40 or data[:2] != b"MZ":
        raise ValueError("not a PE executable")

    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if pe_offset + 24 > len(data) or data[pe_offset:pe_offset + 4] != b"PE\0\0":
        raise ValueError("invalid PE header")
    if struct.unpack_from("<H", data, pe_offset + 4)[0] != 0x014C:
        raise ValueError("expected a 32-bit x86 PE executable")

    optional_size = struct.unpack_from("<H", data, pe_offset + 20)[0]
    optional_offset = pe_offset + 24
    if optional_size < 68 or optional_offset + optional_size > len(data):
        raise ValueError("invalid PE optional header")
    if struct.unpack_from("<H", data, optional_offset)[0] != 0x010B:
        raise ValueError("expected a PE32 optional header")
    if BUILD_MARKER not in data:
        raise ValueError("expected WoW 3.3.5a build 12340")

    return pe_offset + 22, optional_offset + 64


def calculate_pe_checksum(data: bytes, checksum_offset: int) -> int:
    checksum_data = bytearray(data)
    checksum_data[checksum_offset:checksum_offset + 4] = b"\0\0\0\0"
    checksum = 0

    for offset in range(0, len(checksum_data), 2):
        word = checksum_data[offset]
        if offset + 1 < len(checksum_data):
            word |= checksum_data[offset + 1] << 8
        checksum = (checksum + word) & 0xFFFFFFFF
        checksum = (checksum & 0xFFFF) + (checksum >> 16)

    checksum = (checksum & 0xFFFF) + (checksum >> 16)
    checksum = (checksum & 0xFFFF) + (checksum >> 16)
    return (checksum & 0xFFFF) + len(checksum_data)


def _write_atomic(path: Path, data: bytes) -> None:
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=path.name + ".", suffix=".tmp", dir=path.parent
    )
    try:
        with os.fdopen(descriptor, "wb") as temporary:
            temporary.write(data)
            temporary.flush()
            os.fsync(temporary.fileno())
        if path.exists():
            shutil.copymode(path, temporary_name)
        else:
            os.chmod(temporary_name, 0o644)
        os.replace(temporary_name, path)
    finally:
        if os.path.exists(temporary_name):
            os.unlink(temporary_name)


def patch_executable(path: Path) -> bool:
    path = Path(path)
    original = path.read_bytes()
    characteristics_offset, checksum_offset = pe_offsets(original)
    characteristics = struct.unpack_from("<H", original, characteristics_offset)[0]
    if characteristics & LARGE_ADDRESS_AWARE:
        return False

    backup = Path(str(path) + ".pre-laa")
    if backup.exists():
        if backup.read_bytes() != original:
            raise ValueError(f"refusing to overwrite mismatched backup: {backup}")
    else:
        shutil.copy2(path, backup)

    patched = bytearray(original)
    struct.pack_into(
        "<H", patched, characteristics_offset, characteristics | LARGE_ADDRESS_AWARE
    )
    struct.pack_into("<I", patched, checksum_offset, 0)
    struct.pack_into(
        "<I",
        patched,
        checksum_offset,
        calculate_pe_checksum(patched, checksum_offset),
    )
    _write_atomic(path, patched)

    if path.read_bytes() != patched:
        shutil.copy2(backup, path)
        raise OSError("patched executable verification failed; backup restored")
    return True


def restore_legacy_mount_patch(path: Path) -> bool:
    """Restore the stock dismount routine in a client modified by the retired mech workaround."""
    path = Path(path)
    original = path.read_bytes()
    pe_offsets(original)          # validates the PE and the build marker, or raises

    end = MOUNT_CANCEL_OFFSET + len(MOUNT_CANCEL_ORIGINAL)
    if end > len(original):
        raise ValueError("executable is too small to contain the mount-cancel routine")

    present = original[MOUNT_CANCEL_OFFSET:end]
    if present == MOUNT_CANCEL_ORIGINAL:
        return False
    if present != MOUNT_CANCEL_PATCHED:
        raise ValueError(
            f"unexpected bytes at 0x{MOUNT_CANCEL_OFFSET:X}: {present.hex()} "
            f"(expected stock {MOUNT_CANCEL_ORIGINAL.hex()} or legacy {MOUNT_CANCEL_PATCHED.hex()})"
        )

    backup = Path(str(path) + ".pre-moba")
    if not backup.exists():
        raise ValueError(f"cannot reverse legacy mount patch without its backup: {backup}")
    backup_bytes = backup.read_bytes()
    if backup_bytes[MOUNT_CANCEL_OFFSET:end] != MOUNT_CANCEL_ORIGINAL:
        raise ValueError(f"legacy mount backup does not contain the stock routine: {backup}")

    restored = bytearray(original)
    restored[MOUNT_CANCEL_OFFSET:end] = MOUNT_CANCEL_ORIGINAL
    _, checksum_offset = pe_offsets(restored)
    struct.pack_into("<I", restored, checksum_offset, 0)
    struct.pack_into(
        "<I", restored, checksum_offset, calculate_pe_checksum(restored, checksum_offset)
    )
    _write_atomic(path, restored)

    if path.read_bytes() != restored:
        shutil.copy2(backup, path)
        raise OSError("mount-patch reversal failed; backup restored")
    return True


def awesome_patch_state(data: bytes) -> str:
    states = []
    for offset, stock, patched in AWESOME_PATCHES:
        present = data[offset:offset + len(stock)]
        if present == stock:
            states.append("stock")
        elif present == patched:
            states.append("patched")
        else:
            raise ValueError(
                f"unexpected Awesome WotLK bytes at 0x{offset:X}: {present.hex()}"
            )
    if len(set(states)) != 1:
        raise ValueError("partial Awesome WotLK patch detected")
    return states[0]


def validate_awesome_dll(data: bytes) -> None:
    if hashlib.sha256(data).hexdigest() != AWESOME_DLL_SHA256:
        raise ValueError("AwesomeWotlkLib.dll does not match the pinned MOBA build")
    if len(data) < 0x40 or data[:2] != b"MZ":
        raise ValueError("AwesomeWotlkLib.dll is not a PE executable")
    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if (
        pe_offset + 26 > len(data)
        or data[pe_offset:pe_offset + 4] != b"PE\0\0"
        or struct.unpack_from("<H", data, pe_offset + 4)[0] != 0x014C
        or struct.unpack_from("<H", data, pe_offset + 24)[0] != 0x010B
    ):
        raise ValueError("AwesomeWotlkLib.dll must be PE32 x86/i386")
    if any(marker not in data for marker in AWESOME_DLL_API_MARKERS):
        raise ValueError("AwesomeWotlkLib.dll is missing the C_MobaPing API")


def install_awesome_dll(wow_exe: Path) -> bool:
    source = AWESOME_DLL.read_bytes()
    validate_awesome_dll(source)
    target = Path(wow_exe).parent / AWESOME_DLL_NAME
    target_data = target.read_bytes() if target.exists() else None
    if target_data == source:
        return False
    if target_data is not None:
        backup = Path(str(target) + ".pre-awesome")
        if (
            backup.exists()
            and backup.read_bytes() != target_data
            and hashlib.sha256(target_data).hexdigest() not in AWESOME_DLL_PREVIOUS_SHA256
        ):
            raise ValueError(f"refusing to overwrite unknown existing DLL: {target}")
        if not backup.exists():
            shutil.copy2(target, backup)
    _write_atomic(target, source)
    if target.read_bytes() != source:
        raise OSError("AwesomeWotlkLib.dll installation verification failed")
    return True


def patch_awesome_wotlk(path: Path) -> bool:
    path = Path(path)
    original = path.read_bytes()
    _, checksum_offset = pe_offsets(original)
    state = awesome_patch_state(original)
    if state == "patched":
        install_awesome_dll(path)
        return False

    backup = Path(str(path) + ".pre-awesome")
    if backup.exists():
        if backup.read_bytes() != original:
            raise ValueError(f"refusing to overwrite mismatched backup: {backup}")
    else:
        shutil.copy2(path, backup)
    install_awesome_dll(path)

    patched = bytearray(original)
    for offset, _, replacement in AWESOME_PATCHES:
        patched[offset:offset + len(replacement)] = replacement
    struct.pack_into("<I", patched, checksum_offset, 0)
    struct.pack_into(
        "<I", patched, checksum_offset, calculate_pe_checksum(patched, checksum_offset)
    )
    _write_atomic(path, patched)

    if path.read_bytes() != patched:
        shutil.copy2(backup, path)
        raise OSError("Awesome WotLK patch verification failed; backup restored")
    return True


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Patch WoW 3.3.5a build 12340 with Awesome WotLK, Large Address Aware, and "
                    "reverse the retired Ferrailleuse mount workaround when present."
    )
    parser.add_argument("wow_exe", nargs="?", type=Path, default=DEFAULT_WOW)
    args = parser.parse_args()

    try:
        mount_restored = restore_legacy_mount_patch(args.wow_exe)
        laa_changed = patch_executable(args.wow_exe)
        awesome_changed = patch_awesome_wotlk(args.wow_exe)
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    print(f"LAA {'enabled' if laa_changed else 'already enabled'}: {args.wow_exe}")
    print(f"Stock dismount {'restored' if mount_restored else 'already active'}: {args.wow_exe}")
    print(f"Awesome WotLK {'patched' if awesome_changed else 'already active'}: {args.wow_exe}")
    print(f"Awesome DLL installed: {args.wow_exe.parent / AWESOME_DLL_NAME}")
    if laa_changed:
        print(f"Backup: {args.wow_exe}.pre-laa")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
