// Optional browser QA: NODE_PATH=<Playwright node_modules> node launcher/test_ui_layout.js
// Runs the shipped UI with a fake native bridge; no download or game launch can occur.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const http = require('node:http');
const path = require('node:path');
const { chromium } = require('playwright');

const root = path.join(__dirname, 'ui');
const output = path.join(__dirname, 'dist/da-preview');
const types = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.png': 'image/png', '.ttf': 'font/ttf' };

(async () => {
  const config = JSON.parse(await fs.readFile(path.join(__dirname, 'src-tauri/tauri.conf.json'), 'utf8'));
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
      const file = path.resolve(root, `.${pathname === '/' ? '/index.html' : pathname}`);
      if (!file.startsWith(root + path.sep)) throw new Error('Outside UI');
      const bytes = await fs.readFile(file);
      response.writeHead(200, { 'Content-Type': types[path.extname(file)] || 'application/octet-stream', 'Content-Security-Policy': config.app.security.csp });
      response.end(bytes);
    } catch {
      response.writeHead(404).end();
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch();
  await fs.mkdir(output, { recursive: true });
  try {
    for (const language of ['fr', 'en']) {
      for (const [width, height] of [[960, 680], [1280, 800], [1600, 1000]]) {
        const page = await browser.newPage({ viewport: { width, height }, reducedMotion: 'reduce' });
        const errors = [];
        page.on('pageerror', error => errors.push(error.message));
        await page.route('**/*', route => route.request().url().startsWith(origin) ? route.continue() : route.abort());
        await page.addInitScript(language => {
          localStorage.setItem('rivals-launcher-language', language);
          localStorage.setItem('moba-client-dir', 'C:\\Games\\Rivals Beyond');
          window.__TAURI__ = {
            core: { invoke: async (command, args) => {
              if (command === 'client_status') return { state: 'ready', local_version: '2026.9.14.2', remote_version: '2026.9.14.2', available_locales: ['frFR', 'enUS'], installed_locales: [args.locale] };
              if (command === 'launcher_news') return { items: [
                ['le-seuil', 'Le Seuil est ouvert', 'The Threshold is open'],
                ['incarnations', 'Les incarnations du Seuil', 'The incarnations of the Threshold'],
                ['playtest', 'Vos retours du playtest', 'Your playtest feedback'],
              ].map(([slug, fr, en]) => ({ slug, locale: language, title: language === 'fr' ? fr : en, summary: language === 'fr' ? 'Actualité de démonstration pour vérifier la mise en page du launcher.' : 'Demonstration article for checking the launcher layout.', publishedAt: '2026-09-14T08:00:00Z' })) };
              return null;
            } },
            event: { listen: async () => () => {} },
          };
        }, language);
        await page.goto(origin);
        await page.waitForFunction(() => document.querySelectorAll('.news-card').length === 3 && !document.querySelector('#primary-action').disabled);
        await page.evaluate(() => document.fonts.ready);
        assert.equal(await page.evaluate(() => document.fonts.check('600 20px Spectral')), true);
        assert(await page.locator('#primary-action').evaluate(el => parseFloat(getComputedStyle(el).transitionDuration) < 0.001), 'Reduced motion must disable control transitions');
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth), width);
        assert.equal(await page.evaluate(() => document.documentElement.scrollHeight), height);
        for (const state of ['install_required', 'update_available', 'incomplete', 'ready', 'updating', 'game_running', 'launcher_updating', 'error']) {
          await page.evaluate(state => {
            clearError(); setBusy(false);
            renderState({ state, local_version: '2026.9.14.1', remote_version: '2026.9.14.2', current_version: '0.3.10', version: '0.3.11' });
            if (state === 'updating' || state === 'launcher_updating') setBusy(true);
            if (state === 'updating') updateProgress({ phase: 'download', path: 'Data/PATCH-Z.MPQ', bytes_total: 1_000_000_000, bytes_done: 450_000_000 });
            if (state === 'error') showError({ code: 'network', message: 'Connection interrupted (UI test).', retryable: true, detail: 'Connection interrupted (UI test).' }, 'update');
          }, state);
          const action = page.locator('#primary-action');
          const box = await action.boundingBox();
          assert(box && box.x >= 0 && box.y >= 0 && box.x + box.width <= width && box.y + box.height <= height, `${language} ${width} ${state}: action clipped`);
          assert((await action.innerText()).trim());
          assert.equal(await action.isDisabled(), ['updating', 'game_running', 'launcher_updating'].includes(state));
          if (state === 'error') assert(await page.locator('#error-banner').isVisible());
          if (language === 'fr' && width === 1280 && ['ready', 'updating', 'error'].includes(state)) {
            await page.screenshot({ path: path.join(output, `${language}-${width}-${state}.png`) });
          }
        }
        await page.locator('#settings').click();
        assert(await page.locator('#settings-dialog').isVisible());
        const dialog = await page.locator('#settings-dialog').boundingBox();
        assert(dialog && dialog.y >= 0 && dialog.y + dialog.height <= height, 'Settings leave the window');
        if (language === 'fr' && width === 1280) await page.screenshot({ path: path.join(output, 'settings.png') });
        await page.keyboard.press('Escape');
        assert.equal(await page.locator('#settings-dialog').isVisible(), false);
        await page.locator('[data-scroll="news"]').click();
        const news = await page.locator('#news').boundingBox();
        const dock = await page.locator('.action-dock').boundingBox();
        assert(news && dock && news.y + news.height <= dock.y + 1, 'News cannot be reached above the dock');
        assert.deepEqual(errors, []);
        await page.close();
      }
    }
    console.log('Launcher layout: FR/EN, three window sizes, eight states and settings passed.');
  } finally {
    await browser.close();
    server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
