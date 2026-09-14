const assert = require('node:assert/strict');
const { host, tick } = require('./test_ui_languages.js');
const article = (locale, title) => ({slug:'le-seuil-est-ouvert', locale, title, summary:'<b>API text</b>', publishedAt:'2026-09-12T07:00:00+00:00'});
const feed = (locale) => ({items:[article(locale, locale === 'en' ? 'The Threshold is open' : 'Le Seuil est ouvert')]});
(async () => {
  const h = host({}, 'enUS', {}, async ({locale}) => feed(locale === 'frFR' ? 'fr' : 'en'));
  await tick();
  let card = h.node('news-grid').children[0];
  assert.equal(card.href, 'https://rivalsbeyond.com/en/news/le-seuil-est-ouvert');
  assert.equal(card.children[1].children[0].textContent, 'The Threshold is open');
  assert.equal(card.children[1].children[1].textContent, '<b>API text</b>');
  assert.equal(h.node('hero-cta').href, 'https://rivalsbeyond.com/en/news');
  h.node('launcher-language').value = 'fr';
  await h.node('launcher-language').handlers.change(); await tick();
  card = h.node('news-grid').children[0];
  assert.equal(card.href, 'https://rivalsbeyond.com/fr/news/le-seuil-est-ouvert');
  assert.equal(card.children[1].children[0].textContent, 'Le Seuil est ouvert');
  assert(h.storage.has('rivals-api-news-v1-fr'));
  assert(h.storage.has('rivals-api-news-v1-en'));
  assert.equal(h.run('renderNews({items:[]})'), true);
  assert.equal(h.node('news-grid').children.length, 0);
  assert.equal(h.node('news-status').textContent, 'Aucune actualité publiée pour le moment.');
  assert.equal(h.run(`renderNews(${JSON.stringify(feed('en'))})`), false);
  const bad = feed('fr'); bad.items[0].slug = '../register';
  assert.equal(h.run(`renderNews(${JSON.stringify(bad)})`), false);

  const offline = host({'rivals-api-news-v1-fr':JSON.stringify(feed('fr'))}); await tick();
  assert.equal(offline.node('news-grid').children.length, 0);
  assert.equal(offline.node('news-status').textContent, 'News is unavailable at the moment.');
  const cached = host({'rivals-api-news-v1-en':JSON.stringify(feed('en'))}); await tick();
  assert.equal(cached.node('news-grid').children.length, 1);
  assert.equal(cached.node('news-status').textContent, 'Saved news — connection unavailable.');
  const wrongCache = host({'rivals-api-news-v1-en':JSON.stringify(feed('fr'))}); await tick();
  assert.equal(wrongCache.node('news-grid').children.length, 0);
  assert(!wrongCache.storage.has('rivals-api-news-v1-en'));

  let finishEnglish;
  const delayed = host({}, 'enUS', {}, ({locale}) => locale === 'enUS'
    ? new Promise((resolve) => { finishEnglish = resolve; }) : Promise.resolve(feed('fr')));
  await tick();
  delayed.node('launcher-language').value = 'fr';
  await delayed.node('launcher-language').handlers.change(); await tick();
  finishEnglish(feed('en')); await tick();
  assert.equal(delayed.node('news-grid').children[0].children[1].children[0].textContent, 'Le Seuil est ouvert');
  assert(!delayed.storage.has('rivals-api-news-v1-en'));
  console.log('Backend news language, cache and delayed-response checks passed.');
})();
