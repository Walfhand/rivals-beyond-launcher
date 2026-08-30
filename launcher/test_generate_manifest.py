#!/usr/bin/env python3
import hashlib
import struct
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import generate_manifest


def fake_pe(machine: int = 0x014C, magic: int = 0x010B) -> bytes:
    data = bytearray(1024)
    pe_offset = 0x80
    optional_offset = pe_offset + 24
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, pe_offset)
    data[pe_offset:pe_offset + 4] = b"PE\0\0"
    struct.pack_into("<H", data, pe_offset + 4, machine)
    struct.pack_into("<H", data, pe_offset + 20, 0x00E0)
    struct.pack_into("<H", data, optional_offset, magic)
    return bytes(data)


def fake_wow(*, laa: bool = True, stock_dismount: bool = True, awesome: bool = True) -> bytes:
    data = bytearray(generate_manifest.patch_wow_laa.MOUNT_CANCEL_OFFSET + 0x100)
    data[:len(fake_pe())] = fake_pe()
    marker = generate_manifest.patch_wow_laa.BUILD_MARKER
    data[0x200:0x200 + len(marker)] = marker
    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if laa:
        characteristics = struct.unpack_from("<H", data, pe_offset + 22)[0]
        struct.pack_into(
            "<H",
            data,
            pe_offset + 22,
            characteristics | generate_manifest.patch_wow_laa.LARGE_ADDRESS_AWARE,
        )
    start = generate_manifest.patch_wow_laa.MOUNT_CANCEL_OFFSET
    routine = (
        generate_manifest.patch_wow_laa.MOUNT_CANCEL_ORIGINAL
        if stock_dismount
        else generate_manifest.patch_wow_laa.MOUNT_CANCEL_PATCHED
    )
    data[start:start + len(routine)] = routine
    for offset, stock, patched in generate_manifest.patch_wow_laa.AWESOME_PATCHES:
        value = patched if awesome else stock
        data[offset:offset + len(value)] = value
    return bytes(data)


def make_client(root: Path) -> None:
    files = {
        "Wow.exe": fake_wow(),
        "Data/common.MPQ": b"common",
        "Data/patch-C.mpq": b"HD creatures",
        "Data/patch-E.mpq": b"HD lights",
        "Data/patch-P.mpq": b"HD spells",
        "Data/PATCH-Z.MPQ": b"all custom content",
        "Data/frFR/backup-frFR.MPQ": b"locale backup",
        "Data/frFR/locale-frFR.MPQ": b"locale",
        "Data/frFR/realmlist.wtf": b"set realmlist moba.example",
        "Data/frFR/WTF/DefaultBindings.wtf": b"bindings",
        "d3d9.dll": fake_pe(),
        "AwesomeWotlkLib.dll": generate_manifest.patch_wow_laa.AWESOME_DLL.read_bytes(),
        "Interface/AddOns/MobaLevel1PVP/MobaLevel1PVP.toc": b"addon",
    }
    for relative, content in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)


class GenerateManifestTest(unittest.TestCase):
    def test_manifest_is_deterministic_and_excludes_local_state(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "client"
            make_client(root)

            excluded = {
                "Cache/WDB/cache.bin": b"cache",
                "Errors/crash.dmp": b"crash",
                "Logs/FrameXML.log": b"log",
                "WTF/Account/private.wtf": b"private",
                "Data/World/Maps/lastdivide/loose.adt": b"developer copy",
                "Data/patch-4.MPQ.orig": b"backup",
                "Data/frFR/Documentation/ReadMe.html": b"obsolete manual",
                "Wow.exe.pre-laa": b"backup",
                "Repair.exe": b"obsolete repair tool",
                "d3d11.dll": b"unused DXVK DLL",
                "download.part": b"partial",
            }
            for relative, content in excluded.items():
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)

            manifest = generate_manifest.build_manifest(
                root,
                version="2026.7.24",
                objects_url=generate_manifest.DEFAULT_OBJECTS_URL,
            )

            paths = [entry["path"] for entry in manifest["files"]]
            self.assertEqual(paths, sorted(paths, key=str.casefold))
            expected = [
                "Data/common.MPQ",
                "Data/patch-C.mpq",
                "Data/patch-E.mpq",
                "Data/patch-P.mpq",
                "Data/PATCH-Z.MPQ",
                "Data/frFR/backup-frFR.MPQ",
                "Data/frFR/locale-frFR.MPQ",
                "Data/frFR/WTF/DefaultBindings.wtf",
                "AwesomeWotlkLib.dll",
                "d3d9.dll",
                "Interface/AddOns/MobaLevel1PVP/MobaLevel1PVP.toc",
                "Wow.exe",
            ]
            self.assertEqual(paths, sorted(expected, key=str.casefold))
            self.assertEqual(manifest["schema_version"], 1)
            self.assertEqual(manifest["client_version"], "2026.7.24")
            self.assertEqual(manifest["sequence"], 1)
            self.assertEqual(
                manifest["object_base_url"],
                generate_manifest.DEFAULT_OBJECTS_URL,
            )
            self.assertEqual(manifest["file_count"], len(paths))
            self.assertEqual(
                manifest["total_size"],
                sum(entry["size"] for entry in manifest["files"]),
            )
            common = next(
                entry for entry in manifest["files"]
                if entry["path"] == "Data/common.MPQ"
            )
            self.assertEqual(common["sha256"], hashlib.sha256(b"common").hexdigest())
            self.assertEqual(
                manifest,
                generate_manifest.build_manifest(
                    root,
                    version="2026.7.24",
                    objects_url=generate_manifest.DEFAULT_OBJECTS_URL,
                ),
            )

            output = Path(directory) / "manifest.json"
            generate_manifest.write_manifest(output, manifest)
            first = output.read_bytes()
            generate_manifest.write_manifest(output, manifest)
            self.assertEqual(output.read_bytes(), first)

    def test_rejects_legacy_custom_patches(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "client"
            make_client(root)

            for relative in (
                "Data/patch-4.MPQ",
                "Data/frFR/patch-frFR-4.MPQ",
            ):
                legacy = root / relative
                legacy.parent.mkdir(parents=True, exist_ok=True)
                legacy.write_bytes(b"legacy")
                with self.subTest(relative=relative), self.assertRaisesRegex(
                    ValueError, "legacy custom patch"
                ):
                    generate_manifest.build_manifest(
                        root, "1", generate_manifest.DEFAULT_OBJECTS_URL
                    )
                legacy.unlink()

    def test_rejects_symlinks_and_case_insensitive_collisions(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "client"
            make_client(root)
            secret = base / "secret"
            secret.write_bytes(b"do not publish")
            (root / "Data/secret").symlink_to(secret)

            with self.assertRaisesRegex(ValueError, "symlink"):
                generate_manifest.build_manifest(
                    root, "1", generate_manifest.DEFAULT_OBJECTS_URL
                )

            (root / "Data/secret").unlink()
            with self.assertRaisesRegex(ValueError, "unsafe Windows path"):
                generate_manifest._validate_windows_path(Path("Data/patch?.MPQ"))

            if sys.platform != "win32":
                (root / "Data/Patch-X.MPQ").write_bytes(b"first")
                (root / "Data/patch-x.mpq").write_bytes(b"second")

                with self.assertRaisesRegex(ValueError, "case-insensitive"):
                    generate_manifest.build_manifest(
                        root, "1", generate_manifest.DEFAULT_OBJECTS_URL
                    )

                (root / "Data/Patch-X.MPQ").unlink()
                (root / "Data/patch-x.mpq").unlink()
            (root / "Wow.exe").write_bytes(
                b"MZ" + generate_manifest.patch_wow_laa.BUILD_MARKER
            )
            with self.assertRaisesRegex(ValueError, "PE"):
                generate_manifest.build_manifest(
                    root, "1", generate_manifest.DEFAULT_OBJECTS_URL
                )

    def test_accepts_only_official_objects_origin_with_trailing_slash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "client"
            make_client(root)

            manifest = generate_manifest.build_manifest(
                root, "1", generate_manifest.DEFAULT_OBJECTS_URL
            )
            self.assertEqual(
                manifest["object_base_url"], generate_manifest.DEFAULT_OBJECTS_URL
            )

            invalid_urls = (
                "http://moba-data.nbg1.your-objectstorage.com/client/objects/",
                "https://example.com/client/objects/",
                "https://user@moba-data.nbg1.your-objectstorage.com/client/objects/",
                "https://@moba-data.nbg1.your-objectstorage.com/client/objects/",
                "https://moba-data.nbg1.your-objectstorage.com:443/client/objects/",
                "https://moba-data.nbg1.your-objectstorage.com:/client/objects/",
                "https://moba-data.nbg1.your-objectstorage.com/client/objects/?key=value",
                "https://moba-data.nbg1.your-objectstorage.com/client/objects/#fragment",
                "https://moba-data.nbg1.your-objectstorage.com/client/objects",
            )
            for url in invalid_urls:
                with self.subTest(url=url), self.assertRaisesRegex(
                    ValueError, "official HTTPS"
                ):
                    generate_manifest.build_manifest(root, "1", url)

    def test_rejects_d3d9_that_is_not_pe32_x86(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "client"
            make_client(root)

            invalid_dlls = (
                ("x64", fake_pe(machine=0x8664, magic=0x020B)),
                ("PE32+", fake_pe(machine=0x014C, magic=0x020B)),
            )
            for kind, content in invalid_dlls:
                (root / "d3d9.dll").write_bytes(content)
                with self.subTest(kind=kind), self.assertRaisesRegex(
                    ValueError, r"d3d9\.dll.*PE32 x86"
                ):
                    generate_manifest.build_manifest(
                        root, "1", generate_manifest.DEFAULT_OBJECTS_URL
                    )

    def test_rejects_wow_without_large_address_aware(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "client"
            make_client(root)
            (root / "Wow.exe").write_bytes(fake_wow(laa=False))

            with self.assertRaisesRegex(ValueError, "Large Address Aware"):
                generate_manifest.build_manifest(
                    root, "1", generate_manifest.DEFAULT_OBJECTS_URL
                )

    def test_rejects_wow_without_awesome_wotlk(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "client"
            make_client(root)
            (root / "Wow.exe").write_bytes(fake_wow(awesome=False))

            with self.assertRaisesRegex(ValueError, "Awesome WotLK"):
                generate_manifest.build_manifest(
                    root, "1", generate_manifest.DEFAULT_OBJECTS_URL
                )

    def test_rejects_an_unpinned_awesome_dll(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "client"
            make_client(root)
            (root / "AwesomeWotlkLib.dll").write_bytes(fake_pe())

            with self.assertRaisesRegex(ValueError, "pinned MOBA build"):
                generate_manifest.build_manifest(
                    root, "1", generate_manifest.DEFAULT_OBJECTS_URL
                )

    def test_rejects_the_retired_ferrailleuse_mount_hack(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "client"
            make_client(root)
            (root / "Wow.exe").write_bytes(fake_wow(stock_dismount=False))

            with self.assertRaisesRegex(ValueError, "mount hack"):
                generate_manifest.build_manifest(
                    root, "1", generate_manifest.DEFAULT_OBJECTS_URL
                )


if __name__ == "__main__":
    unittest.main()
