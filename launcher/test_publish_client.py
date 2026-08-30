#!/usr/bin/env python3
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

import publish_client


class PublishClientTest(unittest.TestCase):
    def test_public_403_is_missing_only_when_authenticated_s3_confirms_it(self):
        denied = publish_client.urllib.error.HTTPError(
            "https://example.invalid/2.json", 403, "Forbidden", {}, None
        )
        self.addCleanup(denied.close)
        missing = subprocess.CompletedProcess(
            [], 254, stdout="", stderr="404 Not Found"
        )
        with patch.object(
            publish_client.urllib.request, "urlopen", side_effect=denied
        ), patch.object(publish_client, "_aws", return_value=missing):
            self.assertIsNone(publish_client._read_public_manifest("2.json"))

    def test_changed_source_is_removed_from_s3_before_it_can_be_reused(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "patch.MPQ"
            source.write_bytes(b"known client object")
            sha256 = publish_client._sha256_file(source)
            calls = []

            def aws(arguments, capture=False):
                calls.append(arguments)
                if arguments[:2] == ["s3", "cp"]:
                    replacement = source.with_suffix(".new")
                    replacement.write_bytes(b"changed client data")
                    replacement.replace(source)
                return subprocess.CompletedProcess([], 0)

            with patch.object(publish_client, "_aws", side_effect=aws):
                with self.assertRaisesRegex(OSError, "changed during upload"):
                    publish_client._upload_object(source, sha256)

            self.assertTrue(
                any(arguments[:2] == ["s3api", "delete-object"] for arguments in calls)
            )

    def test_remote_state_allows_first_publish_and_identical_retry_only(self):
        def signed(sequence, signature="00"):
            return json.dumps(
                {
                    "payload": json.dumps({"sequence": sequence}),
                    "signature": signature,
                }
            ).encode()

        local = signed(2)
        cases = (
            ("first publish", {}, False, None),
            ("identical retry", {"stable.json": local, "2.json": local}, True, None),
            ("rollback", {"stable.json": signed(3)}, None, "rollback"),
            (
                "stable replacement",
                {"stable.json": signed(2, "ff")},
                None,
                "different signed content",
            ),
            (
                "sequence replacement",
                {"stable.json": signed(1), "2.json": signed(2, "ff")},
                None,
                "already exists with different content",
            ),
        )
        for name, remote, expected, error in cases:
            with self.subTest(name=name), patch.object(
                publish_client,
                "_read_public_manifest",
                side_effect=remote.get,
            ):
                if error:
                    with self.assertRaisesRegex(ValueError, error):
                        publish_client._check_remote_state(2, local)
                else:
                    self.assertEqual(
                        publish_client._check_remote_state(2, local),
                        expected,
                    )

    def test_s3_object_is_reused_only_when_size_and_hash_metadata_match(self):
        sha256 = "ab" * 32

        def response(size, metadata, returncode=0, stderr=""):
            return subprocess.CompletedProcess(
                [],
                returncode,
                stdout=json.dumps(
                    {"ContentLength": size, "Metadata": metadata}
                ),
                stderr=stderr,
            )

        with patch.object(
            publish_client,
            "_aws",
            return_value=response(42, {"sha256": sha256}),
        ):
            self.assertTrue(publish_client._object_exists(sha256, 42))

        with patch.object(
            publish_client,
            "_aws",
            return_value=response(41, {"sha256": sha256}),
        ):
            self.assertFalse(publish_client._object_exists(sha256, 42))

        with patch.object(
            publish_client,
            "_aws",
            return_value=response(0, {}, returncode=254, stderr="404 Not Found"),
        ):
            self.assertFalse(publish_client._object_exists(sha256, 42))


if __name__ == "__main__":
    unittest.main()
