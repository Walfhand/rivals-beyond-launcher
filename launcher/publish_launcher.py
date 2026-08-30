#!/usr/bin/env python3
"""Publish one immutable signed NSIS launcher release and advance stable.json."""

import argparse
import base64
import binascii
import hashlib
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

PROFILE = "moba-s3"
ENDPOINT = "https://nbg1.your-objectstorage.com"
BUCKET = "moba-data"
RELEASE_PREFIX = "launcher/releases"
PUBLIC_ORIGIN = "https://moba-data.nbg1.your-objectstorage.com"
LATEST_KEY = f"{RELEASE_PREFIX}/stable.json"
LATEST_URL = f"{PUBLIC_ORIGIN}/{LATEST_KEY}"
MAX_LATEST_SIZE = 1024 * 1024
DEFAULT_OUTPUT = Path(__file__).resolve().parent / "dist/launcher-latest.json"
VERSION_RE = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def parse_version(value: str) -> tuple[int, int, int]:
    match = VERSION_RE.fullmatch(value)
    if not match:
        raise ValueError("launcher version must be plain MAJOR.MINOR.PATCH SemVer")
    return tuple(map(int, match.groups()))


def validate_version_advance(current: str | None, candidate: str) -> None:
    candidate_version = parse_version(candidate)
    if current is not None and candidate_version < parse_version(current):
        raise ValueError(f"launcher rollback blocked: stable is {current}, local is {candidate}")


def build_latest_document(
    *,
    version: str,
    artifact_name: str,
    signature: str,
    notes: str,
    published_at: str,
) -> dict:
    parse_version(version)
    if (
        Path(artifact_name).name != artifact_name
        or not artifact_name.isascii()
        or not artifact_name.lower().endswith("-setup.exe")
        or version not in artifact_name
    ):
        raise ValueError("launcher artifact must be the versioned ASCII NSIS setup executable")
    if not signature or len(signature) > 4096 or any(char.isspace() for char in signature):
        raise ValueError("launcher signature is invalid")
    try:
        base64.b64decode(signature, validate=True)
    except (binascii.Error, ValueError) as error:
        raise ValueError("launcher signature is invalid base64") from error
    if len(notes) > 4000 or any(char in notes for char in "\x00\r"):
        raise ValueError("launcher release notes are invalid")
    key = f"{RELEASE_PREFIX}/{version}/{artifact_name}"
    return {
        "version": version,
        "notes": notes,
        "pub_date": published_at,
        "platforms": {
            "windows-x86_64": {
                "signature": signature,
                "url": f"{PUBLIC_ORIGIN}/{key}",
            }
        },
    }


def _aws(arguments: list[str], profile: str | None, capture: bool = False):
    command = ["aws", "--endpoint-url", ENDPOINT]
    if profile:
        command.extend(("--profile", profile))
    return subprocess.run(
        [*command, *arguments],
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        text=True,
        check=False,
    )


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _head(key: str, profile: str | None) -> dict | None:
    result = _aws(
        [
            "s3api",
            "head-object",
            "--bucket",
            BUCKET,
            "--key",
            key,
            "--output",
            "json",
        ],
        profile,
        capture=True,
    )
    if result.returncode == 0:
        return json.loads(result.stdout)
    if any(value in result.stderr for value in ("404", "Not Found", "NoSuchKey")):
        return None
    raise OSError(result.stderr.strip() or f"S3 lookup failed: {key}")


def _upload_immutable(source: Path, key: str, content_type: str, profile: str | None) -> None:
    size = source.stat().st_size
    sha256 = _sha256(source)
    current = _head(key, profile)
    if current is not None:
        if (
            current.get("ContentLength") == size
            and current.get("Metadata", {}).get("sha256") == sha256
        ):
            return
        raise ValueError(f"immutable launcher release already exists with different content: {key}")
    result = _aws(
        [
            "s3",
            "cp",
            str(source),
            f"s3://{BUCKET}/{key}",
            "--metadata",
            f"sha256={sha256}",
            "--cache-control",
            "public,max-age=31536000,immutable",
            "--content-type",
            content_type,
            "--no-progress",
        ],
        profile,
    )
    if result.returncode:
        raise OSError(f"launcher upload failed: {key}")
    if source.stat().st_size != size or _sha256(source) != sha256:
        raise OSError(f"launcher artifact changed during upload: {source}")


def _upload_latest(source: Path, profile: str | None) -> None:
    result = _aws(
        [
            "s3",
            "cp",
            str(source),
            f"s3://{BUCKET}/{LATEST_KEY}",
            "--cache-control",
            "no-cache",
            "--content-type",
            "application/json",
            "--no-progress",
        ],
        profile,
    )
    if result.returncode:
        raise OSError("launcher stable.json upload failed")


def _read_latest(profile: str | None) -> dict | None:
    try:
        with urllib.request.urlopen(LATEST_URL, timeout=30) as response:
            payload = response.read(MAX_LATEST_SIZE + 1)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        if error.code == 403 and _head(LATEST_KEY, profile) is None:
            return None
        raise OSError(f"launcher stable lookup failed: HTTP {error.code}") from error
    except urllib.error.URLError as error:
        raise OSError(f"launcher stable lookup failed: {error.reason}") from error
    if len(payload) > MAX_LATEST_SIZE:
        raise ValueError("remote launcher stable.json is too large")
    try:
        return json.loads(payload)
    except json.JSONDecodeError as error:
        raise ValueError("remote launcher stable.json is invalid") from error


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", type=Path)
    parser.add_argument("signature", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--notes", default="Mise à jour du launcher.")
    parser.add_argument("--profile", default=PROFILE)
    parser.add_argument("--no-profile", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    try:
        artifact = args.artifact.resolve(strict=True)
        signature_path = args.signature.resolve(strict=True)
        if not artifact.is_file() or not signature_path.is_file():
            raise ValueError("launcher artifact and signature must be regular files")
        signature = signature_path.read_text().strip()
        profile = None if args.no_profile else args.profile
        remote = _read_latest(profile)
        validate_version_advance(remote.get("version") if remote else None, args.version)
        published_at = (
            remote.get("pub_date")
            if remote and remote.get("version") == args.version
            else datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
        )
        latest = build_latest_document(
            version=args.version,
            artifact_name=artifact.name,
            signature=signature,
            notes=args.notes,
            published_at=published_at,
        )
        if remote and remote.get("version") == args.version and remote != latest:
            raise ValueError(f"stable version {args.version} already has different content")
        DEFAULT_OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        DEFAULT_OUTPUT.write_text(json.dumps(latest, indent=2) + "\n")
        if args.dry_run:
            print(f"Dry run valid: launcher {args.version}")
            return 0

        release_key = f"{RELEASE_PREFIX}/{args.version}/{artifact.name}"
        _upload_immutable(artifact, release_key, "application/vnd.microsoft.portable-executable", profile)
        _upload_immutable(
            signature_path,
            f"{release_key}.sig",
            "text/plain",
            profile,
        )
        _upload_latest(DEFAULT_OUTPUT, profile)
        print(f"Published launcher {args.version}; stable.json advanced last.")
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
