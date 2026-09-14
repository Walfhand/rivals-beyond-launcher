// Run with node launcher/test_ui_languages.js. Executes the shipped scripts with a small DOM host.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const html = fs.readFileSync(path.join(__dirname, 'ui/index.html'), 'utf8');
class Element {
  constructor(attributes = {}) {
    this.attributes = attributes; this.dataset = {}; this.textContent = ''; this.value = attributes.value || '';
    this.children = []; this.handlers = {}; this.classList = { toggle() {} };
    for (const [key, value] of Object.entries(attributes)) {
      if (key.startsWith('data-')) this.dataset[key.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase())] = value;
    }
  }
  setAttribute(key, value) { this.attributes[key] = value; }
  getAttribute(key) { return this.attributes[key]; }
  addEventListener(event, callback) { this.handlers[event] = callback; }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; this.value = children[0]?.value || ''; }
  showModal() { this.open = true; }
  close() { this.open = false; }
}
function host(saved = {}, nativeLocale = 'enUS', status = {}, fetchNews = null) {
  const nodes = [...html.matchAll(/<[a-z][^>]*>/g)].map(([tag]) => new Element(
    Object.fromEntries([...tag.matchAll(/([\w-]+)="([^"]*)"/g)].map(([, key, value]) => [key, value])),
  ));
  nodes.find((node) => node.attributes.id === 'game-language').options = nodes.filter(
    (node) => ['frFR', 'enUS'].includes(node.attributes.value),
  );
  const storage = new Map(Object.entries(saved));
  const calls = [];
  const document = {
    documentElement: {}, hidden: false, addEventListener() {},
    querySelector: (selector) => nodes.find((n) => n.attributes.id === selector.slice(1)),
    querySelectorAll: (selector) => nodes.filter((n) => selector.startsWith('[') && Object.hasOwn(n.attributes, selector.slice(1, -1))),
    createElement: () => new Element(),
  };
  const context = vm.createContext({
    document, navigator: { language: 'de-DE' }, performance, Intl, console,
    localStorage: { getItem: (k) => storage.get(k) ?? null, setItem: (k, v) => storage.set(k, v), removeItem: (k) => storage.delete(k) },
    window: { matchMedia: () => ({ matches: false }), setInterval() {}, addEventListener() {},
      __TAURI__: { core: { invoke: async (command, args) => {
        calls.push({ command, args });
        if (command === 'system_locale') return nativeLocale;
        if (command === 'launcher_news') {
          if (fetchNews) return fetchNews(args);
          throw new Error('Offline feed');
        }
        if (command === 'client_status') return { state: 'ready', available_locales: ['frFR', 'enUS'], installed_locales: [args.locale], ...status };
        if (command === 'update_client') return { version: '1', changed_files: 1, available_locales: ['frFR','enUS'], installed_locales: ['frFR','enUS'] };
        return null;
      } } },
    },
  });
  for (const file of ['locales.js', 'app.js']) vm.runInContext(fs.readFileSync(path.join(__dirname, 'ui', file), 'utf8'), context);
  return { context, calls, storage, nodes, node: (id) => document.querySelector('#' + id), run: (code) => vm.runInContext(code, context) };
}
const tick = () => new Promise((resolve) => setImmediate(resolve));
if (require.main === module) (async () => {
  const h = host(); await tick();
  for (const value of ['fr', 'fr-FR', 'fr-CA', 'FR_be']) assert.equal(h.run(`chooseLanguage(null, ${JSON.stringify(value)})`), 'fr');
  for (const value of ['en-US', 'de-DE', '', 'français']) assert.equal(h.run(`chooseLanguage(null, ${JSON.stringify(value)})`), 'en');
  assert.equal(h.run('chooseLanguage("fr", "de-DE")'), 'fr');
  assert.equal(h.run('chooseLanguage("en", "fr-FR")'), 'en');
  assert.equal(h.run('chooseLanguage("invalid", "fr-FR")'), 'fr');
  const dictionaries = h.run('LauncherLocales');
  assert.deepEqual(Object.keys(dictionaries.fr).sort(), Object.keys(dictionaries.en).sort());
  for (const key of Object.keys(dictionaries.fr)) {
    assert.deepEqual(dictionaries.fr[key].match(/\{\w+\}/g), dictionaries.en[key].match(/\{\w+\}/g), key);
  }
  assert.equal(h.node('primary-action').textContent, 'Install');
  assert.equal(h.node('hero-title').textContent, 'Enter the Threshold');
  assert.equal(h.context.document.documentElement.lang, 'en');
  h.run('showError({ code: "network", message: "Connexion interrompue", detail: "Lecture réseau", retryable: true }, "update")');
  assert.equal(h.node('error-message').textContent, dictionaries.en.error_network);
  assert(!h.node('error-detail').textContent.includes('Lecture'));
  h.run('updateProgress({ phase: "download", message: "Téléchargement", path: "Data/enUS/locale.MPQ", bytes_total: 100, bytes_done: 20 })');
  assert.equal(h.node('client-status').textContent, 'Downloading: Data/enUS/locale.MPQ');
  for (const state of ['checking', 'install_required', 'update_available', 'incomplete', 'ready', 'updating', 'game_running', 'launcher_updating', 'error']) {
    h.run(`renderState({ state: "${state}", local_version: "1", remote_version: "2", version: "3", current_version: "2" })`);
    assert(h.node('primary-action').textContent);
  }
  assert.equal(h.run('formatBytes(1024)'), '1 KB');
  assert.equal(h.run('formatDuration(30)'), '30 s remaining');
  const french = host({}, 'frFR'); await tick();
  assert.equal(french.context.document.documentElement.lang, 'fr');
  assert.equal(french.node('primary-action').textContent, 'Installer');
  french.node('launcher-language').value = 'en';
  await french.node('launcher-language').handlers.change();
  assert.equal(french.node('primary-action').textContent, 'Install');
  assert.equal(french.storage.get('rivals-launcher-language'), 'en');
  assert.equal(french.node('game-language').value, 'enUS');
  const saved = { 'rivals-launcher-language': 'en', 'moba-client-dir': '/game' };
  const install = host(saved, 'frFR'); await tick();
  assert(!install.calls.some((call) => call.command === 'system_locale'));
  assert.equal(install.calls.find((call) => call.command === 'client_status').args.locale, 'enUS');
  install.node('additional-language').value = 'frFR';
  install.node('add-language').handlers.click(); await tick();
  assert.deepEqual(install.calls.find((call) => call.command === 'update_client').args.additionalLocale, 'frFR');
  assert.equal(install.node('add-language').disabled, true);
  assert.equal(install.storage.get('rivals-game-language'), 'enUS', 'The first successful install fixes the initial game language');
  install.node('launcher-language').value = 'fr'; await install.node('launcher-language').handlers.change();
  assert.equal(install.node('game-language').value, 'enUS', 'Changing the launcher before first play must not request another pack');
  install.node('launcher-language').value = 'en'; await install.node('launcher-language').handlers.change();
  await install.run('play()');
  assert.equal(install.calls.find((call) => call.command === 'launch_game').args.locale, 'enUS');
  install.node('game-language').value = 'frFR';
  await install.node('game-language').handlers.change();
  assert.equal(install.storage.get('rivals-game-language'), 'frFR');
  install.node('launcher-language').value = 'fr'; await install.node('launcher-language').handlers.change();
  install.node('launcher-language').value = 'en'; await install.node('launcher-language').handlers.change();
  assert.equal(install.node('game-language').value, 'frFR');
  const existing = host(saved, 'enUS', { configured_locale: 'frFR' }); await tick();
  assert.equal(existing.storage.get('rivals-game-language'), 'frFR');
  assert.equal(existing.calls.filter((c) => c.command === 'client_status').at(-1).args.locale, 'frFR');
  const legacy = host({ ...saved, 'rivals-game-language': 'frFR' }, 'frFR', { available_locales: ['frFR'] }); await tick();
  const english = legacy.node('game-language').options.find((option) => option.value === 'enUS');
  assert.equal(english.disabled, true, 'An unadvertised language must not be selectable');
  assert.equal(english.hidden, true);
  legacy.run('selectedGameLocale = "enUS"; applyLanguage()');
  assert.equal(english.disabled, true);
  assert.equal(english.hidden, false, 'A saved unavailable choice remains visible until changed');
  h.run("applyLanguage()");
  for (const node of h.nodes) {
    if (node.dataset.i18n) assert.equal(node.textContent, dictionaries.en[node.dataset.i18n], node.dataset.i18n);
    for (const attribute of ['aria-label', 'title', 'placeholder']) {
      const key = node.getAttribute(`data-i18n-${attribute}`);
      if (key) assert.equal(node.getAttribute(attribute), dictionaries.en[key], `${attribute}: ${key}`);
    }
  }
  for (const [, key] of fs.readFileSync(path.join(__dirname, 'ui/app.js'), 'utf8').matchAll(/\bt\("([^"$]+)"/g)) {
    assert.equal(typeof dictionaries.en[key], 'string', `Unknown translation key ${key}`);
  }
  for (const key of Object.keys(dictionaries.en).filter((key) => key.startsWith('error_') && key !== 'error_detail')) {
    const code = key.slice(6);
    h.run(`showError({ code: ${JSON.stringify(code)}, message: "Message français du serveur", detail: "Détail français brut", retryable: true }, "update")`);
    assert.equal(h.node('error-message').textContent, dictionaries.en[key], key);
    assert.equal(h.node('client-status').textContent, dictionaries.en[key], key);
    assert(!h.node('error-detail').textContent.includes('français'), key);
  }
  for (const phase of ['check', 'download', 'retry', 'install']) {
    h.run(`updateProgress({ phase: "${phase}", message: "Message français du serveur", path: "Data/patch.MPQ", delay_seconds: 2, bytes_total: 100, bytes_done: 20 })`);
    assert.equal(h.node('client-status').textContent, h.run(`t("phase_${phase}", { path: "Data/patch.MPQ", seconds: 2 })`));
  }
  h.run('showError("Erreur française sans structure", "update")');
  assert.equal(h.node('error-message').textContent, dictionaries.en.error_unknown);
  h.run('availableLocales = ["frFR", "enUS"]; installedLocales = ["frFR"]; renderLanguagePacks()');
  assert.equal(h.node('installed-languages').textContent, 'Installed languages: French');
  assert.equal(h.node('additional-language').children[0].textContent, 'English');
  await h.run('chooseDirectory()');
  assert.equal(h.calls.find((call) => call.command === 'choose_client_dir').args.locale, 'enUS');
  console.log('Launcher UI language checks passed.');
})();

module.exports = { host, tick };
