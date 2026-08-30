#!/usr/bin/env python3
"""Sign a client manifest with the release Ed25519 key."""

import argparse
import json
import os
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

from generate_manifest import write_manifest


PUBLIC_DER_PREFIX = bytes.fromhex("302a300506032b6570032100")
DEFAULT_OUTPUT = Path(__file__).resolve().parent / "dist/client-manifest.signed.json"


def private_key_path(path: Path) -> Path:
    path = path.resolve()
    metadata = path.stat()
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError("private key must be a regular file")
    if os.name != "nt" and metadata.st_mode & 0o077:
        raise ValueError("private key permissions must be 0600")
    return path


def _openssl(arguments: list[str], payload: bytes | None = None) -> bytes:
    result = subprocess.run(
        ["openssl", *arguments],
        input=payload,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode:
        raise OSError(result.stderr.decode(errors="replace").strip())
    return result.stdout


def public_key(private_key: Path) -> bytes:
    der = _openssl(
        ["pkey", "-in", str(private_key), "-pubout", "-outform", "DER"]
    )
    if not der.startswith(PUBLIC_DER_PREFIX) or len(der) != len(PUBLIC_DER_PREFIX) + 32:
        raise ValueError("private key is not an Ed25519 key")
    return der[len(PUBLIC_DER_PREFIX):]


def signed_document(payload: bytes, private_key: Path) -> dict:
    try:
        json.loads(payload)
        text = payload.decode("utf-8")
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid manifest JSON: {error}") from error
    with tempfile.NamedTemporaryFile() as source:
        source.write(payload)
        source.flush()
        signature = _openssl(
            [
                "pkeyutl",
                "-sign",
                "-rawin",
                "-inkey",
                str(private_key),
                "-in",
                source.name,
            ]
        )
    if len(signature) != 64:
        raise ValueError("OpenSSL returned an invalid Ed25519 signature")
    return {
        "schema_version": 1,
        "payload": text,
        "signature": signature.hex(),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("private_key", type=Path)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    try:
        private_key = private_key_path(args.private_key)
        payload = args.manifest.read_bytes()
        document = signed_document(payload, private_key)
        write_manifest(args.output, document)
        print(f"Signed manifest: {args.output.resolve()}")
        print(f"Public key: {public_key(private_key).hex()}")
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
