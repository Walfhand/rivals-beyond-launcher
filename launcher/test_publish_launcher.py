import unittest
import sys
import urllib.error
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import publish_launcher


class PublishLauncherTest(unittest.TestCase):
    def test_public_403_is_missing_only_when_authenticated_s3_confirms_it(self):
        denied = urllib.error.HTTPError(
            publish_launcher.LATEST_URL, 403, "Forbidden", {}, None
        )
        self.addCleanup(denied.close)
        with patch.object(
            publish_launcher.urllib.request, "urlopen", side_effect=denied
        ), patch.object(publish_launcher, "_head", return_value=None):
            self.assertIsNone(publish_launcher._read_latest("moba-s3"))

    def test_latest_document_points_to_the_immutable_signed_nsis_release(self):
        latest = publish_launcher.build_latest_document(
            version="0.4.0",
            artifact_name="MOBA-Launcher_0.4.0_x64-setup.exe",
            signature="dHJ1c3RlZC1zaWduYXR1cmU=",
            notes="Mise à jour automatique.",
            published_at="2026-08-28T18:00:00Z",
        )

        windows = latest["platforms"]["windows-x86_64"]
        self.assertEqual(latest["version"], "0.4.0")
        self.assertEqual(windows["signature"], "dHJ1c3RlZC1zaWduYXR1cmU=")
        self.assertEqual(
            windows["url"],
            "https://moba-data.nbg1.your-objectstorage.com/launcher/releases/0.4.0/"
            "MOBA-Launcher_0.4.0_x64-setup.exe",
        )

    def test_stable_version_only_moves_forward_or_republishes_identically(self):
        publish_launcher.validate_version_advance(None, "0.3.0")
        publish_launcher.validate_version_advance("0.3.0", "0.4.0")
        publish_launcher.validate_version_advance("0.4.0", "0.4.0")
        with self.assertRaisesRegex(ValueError, "rollback"):
            publish_launcher.validate_version_advance("0.4.0", "0.3.0")

    def test_release_versions_are_plain_semver(self):
        self.assertEqual(publish_launcher.parse_version("12.3.45"), (12, 3, 45))
        for value in ("v1.2.3", "1.2", "1.2.3-beta", "01.2.3"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                publish_launcher.parse_version(value)


if __name__ == "__main__":
    unittest.main()
