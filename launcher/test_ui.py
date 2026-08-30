import json
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent


class LauncherUiTest(unittest.TestCase):
    def test_launcher_uses_the_rivals_beyond_brand(self):
        html = (ROOT / "ui/index.html").read_text()
        script = (ROOT / "ui/app.js").read_text()
        tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())

        self.assertIn("Rivals Beyond", html)
        self.assertIn("Rivals Beyond", script)
        self.assertEqual(tauri["productName"], "Rivals Beyond")
        self.assertEqual(tauri["identifier"], "game.rivalsbeyond.launcher")
        self.assertNotIn("World of Guerilla", html + script)
        self.assertTrue((ROOT / "ui/assets/moba-logo.png").is_file())

    def test_launcher_uses_the_selected_last_divide_art_direction(self):
        style = (ROOT / "ui/style.css").read_text()
        selected_art = (
            ROOT.parent
            / "client-patches/textures/login/rivals_beyond_last_divide_login_selected.png"
        )

        self.assertEqual(
            (ROOT / "ui/assets/moba-background.png").read_bytes(),
            selected_art.read_bytes(),
        )
        self.assertIn("--azure:", style)
        self.assertIn("--scarlet:", style)
        self.assertNotIn("backdrop-filter", style)
        self.assertNotIn("font-family: Inter", style)

    def test_home_has_news_and_one_contextual_primary_action(self):
        html = (ROOT / "ui/index.html").read_text()
        self.assertIn('id="news"', html)
        self.assertIn('id="news-grid"', html)
        self.assertEqual(html.count('id="primary-action"'), 1)
        self.assertNotIn('id="update"', html)
        self.assertNotIn('id="play"', html)

    def test_news_are_remote_signed_cached_and_rendered_as_text(self):
        script = (ROOT / "ui/app.js").read_text()
        self.assertIn('invoke("launcher_news")', script)
        self.assertIn("moba-launcher-news", script)
        self.assertIn("textContent", script)
        self.assertNotIn("innerHTML", script)
        feed = json.loads((ROOT / "news.json").read_text())
        self.assertEqual(feed["schema_version"], 1)
        self.assertEqual(len(feed["items"]), 3)

    def test_launcher_self_update_is_signed_and_runs_before_client_status(self):
        script = (ROOT / "ui/app.js").read_text()
        self.assertIn('invoke("check_launcher_update")', script)
        self.assertIn('invoke("install_launcher_update")', script)
        boot = script.split("async function boot()", 1)[1]
        self.assertLess(boot.index("await checkLauncherUpdate()"), boot.index("await refreshStatus(true)"))
        tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())
        updater = tauri["plugins"]["updater"]
        self.assertTrue(tauri["bundle"]["createUpdaterArtifacts"])
        self.assertEqual(
            updater["pubkey"],
            (ROOT / "updater-public-key.pub").read_text().strip(),
        )
        self.assertEqual(updater["windows"]["installMode"], "passive")

    def test_tauri_and_cargo_versions_stay_aligned(self):
        tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())
        cargo = tomllib.loads((ROOT / "src-tauri/Cargo.toml").read_text())
        self.assertEqual(tauri["version"], cargo["package"]["version"])

    def test_windows_release_verifies_and_publishes_the_signed_updater(self):
        workflow = (ROOT.parent / ".github/workflows/launcher-windows.yml").read_text()
        self.assertIn("TAURI_SIGNING_PRIVATE_KEY", workflow)
        self.assertIn("verify_updater_signature", workflow)
        self.assertIn("actions/upload-artifact@v4", workflow)
        self.assertIn("*-setup.exe.sig", workflow)
        self.assertIn("publish_launcher.py", workflow)
        self.assertLess(
            workflow.index("verify_updater_signature"),
            workflow.rindex("publish_launcher.py"),
        )


if __name__ == "__main__":
    unittest.main()
