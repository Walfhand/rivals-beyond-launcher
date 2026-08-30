#!/usr/bin/env python3
"""Verify, sign and publish the current client to Hetzner Object Storage."""

import argparse
import hashlib
import json
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

import generate_manifest
import sign_manifest


PROFILE = "moba-s3"
ENDPOINT = "https://nbg1.your-objectstorage.com"
BUCKET = "moba-data"
OBJECT_PREFIX = "client/objects/sha256"
MANIFEST_PREFIX = "client/manifests"
PUBLIC_MANIFEST_PREFIX = (
    "https://moba-data.nbg1.your-objectstorage.com/client/manifests"
)
MAX_MANIFEST_BYTES = 16 * 1024 * 1024
SIGNED_OUTPUT = Path(__file__).resolve().parent / "dist/client-manifest.signed.json"
NEWS_SOURCE = Path(__file__).resolve().parent / "news.json"
SIGNED_NEWS_OUTPUT = Path(__file__).resolve().parent / "dist/news.signed.json"
NEWS_KEY = "launcher/news/stable.json"
PUBLIC_KEY = Path(__file__).resolve().parent / "manifest-public-key.hex"


def _aws(arguments: list[str], capture: bool = False) -> subprocess.CompletedProcess:
    return subprocess.run(
        [
            "aws",
            "--profile",
            PROFILE,
            "--endpoint-url",
            ENDPOINT,
            *arguments,
        ],
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        text=True,
        check=False,
    )


def _object_exists(sha256: str, size: int) -> bool:
    result = _aws(
        [
            "s3api",
            "head-object",
            "--bucket",
            BUCKET,
            "--key",
            f"{OBJECT_PREFIX}/{sha256}",
            "--output",
            "json",
        ],
        capture=True,
    )
    if result.returncode == 0:
        response = json.loads(result.stdout)
        return (
            response.get("ContentLength") == size
            and response.get("Metadata", {}).get("sha256") == sha256
        )
    if any(value in result.stderr for value in ("404", "Not Found", "NoSuchKey")):
        return False
    raise OSError(result.stderr.strip() or "S3 head-object failed")


def _source_identity(source: Path) -> tuple[int, int, int, int]:
    stat = source.stat()
    if not source.is_file():
        raise ValueError(f"source is not a regular file: {source}")
    return stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns


def _sha256_file(source: Path) -> str:
    digest = hashlib.sha256()
    with source.open("rb") as stream:
        for block in iter(lambda: stream.read(8 * 1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _upload_object(source: Path, sha256: str) -> None:
    identity = _source_identity(source)
    if _sha256_file(source) != sha256 or _source_identity(source) != identity:
        raise ValueError(f"source changed before upload: {source}")
    result = _aws(
        [
            "s3",
            "cp",
            str(source),
            f"s3://{BUCKET}/{OBJECT_PREFIX}/{sha256}",
            "--metadata",
            f"sha256={sha256}",
            "--cache-control",
            "public,max-age=31536000,immutable",
            "--content-type",
            "application/octet-stream",
            "--no-progress",
        ]
    )
    if result.returncode:
        raise OSError(f"upload failed: {source}")
    try:
        changed = _source_identity(source) != identity
    except (OSError, ValueError):
        changed = True
    if changed:
        cleanup = _aws(
            [
                "s3api",
                "delete-object",
                "--bucket",
                BUCKET,
                "--key",
                f"{OBJECT_PREFIX}/{sha256}",
            ]
        )
        detail = "" if cleanup.returncode == 0 else "; remote cleanup also failed"
        raise OSError(f"source changed during upload: {source}{detail}")


def _upload_manifest(source: Path, key: str, cache_control: str) -> None:
    result = _aws(
        [
            "s3",
            "cp",
            str(source),
            f"s3://{BUCKET}/{key}",
            "--cache-control",
            cache_control,
            "--content-type",
            "application/json",
            "--no-progress",
        ]
    )
    if result.returncode:
        raise OSError(f"manifest upload failed: {key}")


def _read_public_manifest(filename: str) -> bytes | None:
    url = f"{PUBLIC_MANIFEST_PREFIX}/{filename}"
    try:
        with urllib.request.urlopen(url, timeout=30) as response:
            document = response.read(MAX_MANIFEST_BYTES + 1)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        if error.code == 403:
            head = _aws(
                [
                    "s3api",
                    "head-object",
                    "--bucket",
                    BUCKET,
                    "--key",
                    f"{MANIFEST_PREFIX}/{filename}",
                ],
                capture=True,
            )
            if head.returncode and any(
                value in head.stderr for value in ("404", "Not Found", "NoSuchKey")
            ):
                return None
        raise OSError(f"manifest lookup failed: {url}: HTTP {error.code}") from error
    except urllib.error.URLError as error:
        raise OSError(f"manifest lookup failed: {url}: {error.reason}") from error
    if len(document) > MAX_MANIFEST_BYTES:
        raise ValueError(f"remote manifest exceeds {MAX_MANIFEST_BYTES} bytes")
    return document


def _check_remote_state(sequence: int, signed_document: bytes) -> bool:
    stable = _read_public_manifest("stable.json")
    if stable is not None:
        try:
            current_sequence = json.loads(json.loads(stable)["payload"])["sequence"]
        except (TypeError, KeyError, json.JSONDecodeError) as error:
            raise ValueError("remote stable manifest is invalid") from error
        if type(current_sequence) is not int or current_sequence < 1:
            raise ValueError("remote stable manifest has an invalid sequence")
        if current_sequence > sequence:
            raise ValueError(
                "sequence rollback blocked: "
                f"stable is {current_sequence}, local is {sequence}"
            )
        if current_sequence == sequence and stable != signed_document:
            raise ValueError(
                f"stable sequence {sequence} has different signed content"
            )

    existing = _read_public_manifest(f"{sequence}.json")
    if existing is None:
        return False
    if existing != signed_document:
        raise ValueError(
            f"sequence {sequence} already exists with different content "
            f"(remote sha256 {hashlib.sha256(existing).hexdigest()}, "
            f"local sha256 {hashlib.sha256(signed_document).hexdigest()})"
        )
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("client_dir", type=Path)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("private_key", type=Path)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    try:
        manifest = json.loads(args.manifest.read_text())
        fresh = generate_manifest.build_manifest(
            args.client_dir,
            version=manifest["client_version"],
            objects_url=manifest["object_base_url"],
            sequence=manifest["sequence"],
            progress=lambda index, total, path: print(
                f"[verify {index}/{total}] {path}", file=sys.stderr, flush=True
            ),
        )
        if fresh != manifest:
            raise ValueError("client no longer matches the manifest; generate it again")

        private_key = sign_manifest.private_key_path(args.private_key)
        public_key = sign_manifest.public_key(private_key).hex()
        if public_key != PUBLIC_KEY.read_text().strip():
            raise ValueError("private key does not match the launcher public key")
        sign_manifest.write_manifest(
            SIGNED_OUTPUT,
            sign_manifest.signed_document(args.manifest.read_bytes(), private_key),
        )
        sign_manifest.write_manifest(
            SIGNED_NEWS_OUTPUT,
            sign_manifest.signed_document(NEWS_SOURCE.read_bytes(), private_key),
        )
        if args.dry_run:
            print(f"Dry run valid: {manifest['file_count']} files and signed news")
            return 0

        sequence_exists = _check_remote_state(
            manifest["sequence"],
            SIGNED_OUTPUT.read_bytes(),
        )
        root = args.client_dir.resolve()
        for index, entry in enumerate(manifest["files"], 1):
            print(f"[{index}/{manifest['file_count']}] {entry['path']}", flush=True)
            if _object_exists(entry["sha256"], entry["size"]):
                print("  already present", flush=True)
                continue
            _upload_object(root / Path(*entry["path"].split("/")), entry["sha256"])

        sequence_key = f"{MANIFEST_PREFIX}/{manifest['sequence']}.json"
        if not sequence_exists:
            _upload_manifest(
                SIGNED_OUTPUT,
                sequence_key,
                "public,max-age=31536000,immutable",
            )
        _upload_manifest(
            SIGNED_OUTPUT,
            f"{MANIFEST_PREFIX}/stable.json",
            "no-cache",
        )
        _upload_manifest(SIGNED_NEWS_OUTPUT, NEWS_KEY, "no-cache")
        print(f"Published sequence {manifest['sequence']} after all objects.")
    except (KeyError, json.JSONDecodeError, OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
