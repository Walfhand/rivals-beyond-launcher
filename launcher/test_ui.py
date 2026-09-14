import hashlib
import json
from html.parser import HTMLParser
import subprocess
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent


class LauncherUiTest(unittest.TestCase):
    def test_language_preferences_and_download_actions_execute_production_ui(self):
        subprocess.run(["node", str(ROOT / "test_ui_languages.js")], check=True)

    def test_automatic_diagnostics_preference_is_disclosed_and_passed_to_the_game_launch(self):
        html = (ROOT / "ui/index.html").read_text(encoding="utf-8")
        script = (ROOT / "ui/app.js").read_text(encoding="utf-8")
        main = (ROOT / "src-tauri/src/main.rs").read_text(encoding="utf-8")
        self.assertIn('id="diagnostics-enabled" type="checkbox" checked', html)
        self.assertIn('aria-describedby="diagnostics-help"', html)
        self.assertIn('diagnosticsEnabled: diagnosticsToggle.checked', script)
        self.assertIn('localStorage.setItem(DIAGNOSTICS_KEY', script)
        self.assertIn('"MOBA_DIAGNOSTICS_UPLOAD"', main)
        self.assertIn('"MOBA_DIAGNOSTIC_SESSION"', main)

    def test_launcher_uses_the_rivals_beyond_brand(self):
        html = (ROOT / "ui/index.html").read_text(encoding="utf-8")
        script = (ROOT / "ui/app.js").read_text(encoding="utf-8")
        tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))

        self.assertIn("Rivals Beyond", html)
        self.assertIn("Rivals Beyond", script)
        self.assertEqual(tauri["productName"], "Rivals Beyond")
        self.assertEqual(tauri["identifier"], "game.rivalsbeyond.launcher")
        self.assertNotIn("World of Guerilla", html + script)
        self.assertTrue((ROOT / "ui/assets/moba-logo.png").is_file())

    def test_launcher_uses_the_selected_galdric_vespera_duel(self):
        style = (ROOT / "ui/style.css").read_text(encoding="utf-8")
        self.assertEqual(
            hashlib.sha256((ROOT / "ui/assets/moba-background.png").read_bytes()).hexdigest(),
            "17d299dc78d122dc4c77b19379627dcd8c41790375c215b832d65df48005db42",
        )
        self.assertIn('url("assets/moba-background.png")', style)
        self.assertIn("--azure:", style)
        self.assertIn("--scarlet:", style)
        self.assertNotIn("backdrop-filter", style)
        self.assertNotIn("font-family: Inter", style)

    def test_home_has_news_and_one_contextual_primary_action(self):
        html = (ROOT / "ui/index.html").read_text(encoding="utf-8")
        script = (ROOT / "ui/app.js").read_text(encoding="utf-8")
        self.assertIn('id="news"', html)
        self.assertIn('id="news-grid"', html)
        self.assertRegex(html, r'id="hero-cta"[^>]*class="outline-button"')
        self.assertIn('href="https://rivalsbeyond.com/en/news"', html)
        self.assertNotIn('document.querySelector("#hero-cta").addEventListener', script)
        self.assertEqual(html.count('id="primary-action"'), 1)
        self.assertNotIn('id="update"', html)
        self.assertNotIn('id="play"', html)

    def test_account_creation_opens_only_the_official_registration_page(self):
        html = (ROOT / "ui/index.html").read_text(encoding="utf-8")
        main = (ROOT / "src-tauri/src/main.rs").read_text(encoding="utf-8")
        cargo = (ROOT / "src-tauri/Cargo.toml").read_text(encoding="utf-8")
        capability = json.loads((ROOT / "src-tauri/capabilities/default.json").read_text(encoding="utf-8"))

        self.assertIn('href="https://rivalsbeyond.com/register"', html)
        self.assertIn('target="_blank"', html)
        self.assertIn('rel="noreferrer"', html)
        self.assertIn('tauri-plugin-opener', cargo)
        self.assertIn("tauri_plugin_opener::init()", main)
        opener = next(
            permission for permission in capability["permissions"]
            if isinstance(permission, dict)
            and permission["identifier"] == "opener:allow-open-url"
        )
        self.assertEqual(opener["allow"], [{"url": "https://rivalsbeyond.com/*"}])

    def test_news_come_from_localized_backend_and_render_as_text(self):
        subprocess.run(["node", str(ROOT / "test_ui_news.js")], check=True)
        script = (ROOT / "ui/app.js").read_text(encoding="utf-8")
        news = (ROOT / "src-tauri/src/news.rs").read_text(encoding="utf-8")
        main = (ROOT / "src-tauri/src/main.rs").read_text(encoding="utf-8")
        self.assertNotIn("innerHTML", script)
        self.assertNotIn("LauncherNews", script)
        self.assertNotIn("MOBA_NEWS_URL", main)
        self.assertIn("https://api.rivalsbeyond.com/api/v1/news", news)
        self.assertFalse((ROOT / "news.json").exists())

    def test_launcher_self_update_is_signed_and_runs_before_client_status(self):
        script = (ROOT / "ui/app.js").read_text(encoding="utf-8")
        self.assertIn('invoke("check_launcher_update")', script)
        self.assertIn('invoke("install_launcher_update")', script)
        boot = script.split("async function boot()", 1)[1]
        self.assertLess(boot.index("await checkLauncherUpdate()"), boot.index("await refreshStatus(true)"))
        tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
        updater = tauri["plugins"]["updater"]
        self.assertTrue(tauri["bundle"]["createUpdaterArtifacts"])
        self.assertEqual(
            updater["pubkey"],
            (ROOT / "updater-public-key.pub").read_text(encoding="utf-8").strip(),
        )
        self.assertEqual(updater["windows"]["installMode"], "passive")

    def test_windows_build_defaults_to_the_public_game_realm(self):
        main = (ROOT / "src-tauri/src/main.rs").read_text(encoding="utf-8")
        workflow = (ROOT.parent / ".github/workflows/launcher-windows.yml").read_text(encoding="utf-8")
        realm = main.split("const REALM_ADDRESS:", 1)[1].split("};", 1)[0]
        self.assertIn('option_env!("MOBA_REALM_ADDRESS")', realm)
        self.assertIn('None => "moba.rivalsbeyond.com"', realm)
        realm_input = workflow.split("      realm_address:", 1)[1].split("      manifest_url:", 1)[0]
        self.assertIn("default: moba.rivalsbeyond.com", realm_input)
        self.assertIn("MOBA_REALM_ADDRESS: ${{ inputs.realm_address }}", workflow)

    def test_windows_installer_supports_french_and_english_with_english_fallback(self):
        tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
        self.assertEqual(tauri["bundle"]["windows"]["nsis"], {
            "languages": ["English", "French"], "displayLanguageSelector": True,
        })
        self.assertEqual(tauri["bundle"]["shortDescription"], "Install, update and play Rivals Beyond")

    def test_html_has_no_untranslated_visible_copy_outside_the_brand(self):
        class CopyParser(HTMLParser):
            def __init__(self):
                super().__init__()
                self.copy = []

            def handle_data(self, data):
                if any(character.isalpha() for character in data):
                    self.copy.append(data.strip())

        parser = CopyParser()
        parser.feed((ROOT / "ui/index.html").read_text(encoding="utf-8"))
        self.assertEqual(parser.copy, ["Rivals Beyond"])

    def test_windows_installer_bundles_webview2_without_a_separate_download(self):
        tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
        self.assertEqual(
            tauri["bundle"]["windows"]["webviewInstallMode"],
            {"type": "offlineInstaller", "silent": True},
        )

    def test_tauri_and_cargo_versions_stay_aligned(self):
        tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
        cargo = tomllib.loads((ROOT / "src-tauri/Cargo.toml").read_text(encoding="utf-8"))
        self.assertEqual(tauri["version"], cargo["package"]["version"])

    def test_windows_release_verifies_and_publishes_the_signed_updater(self):
        workflow = (ROOT.parent / ".github/workflows/launcher-windows.yml").read_text(encoding="utf-8")
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
