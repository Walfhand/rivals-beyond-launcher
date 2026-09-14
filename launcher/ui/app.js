const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke;
const listen = tauri?.event?.listen;

const pathInput = document.querySelector("#client-path");
const chooseButton = document.querySelector("#choose");
const repairButton = document.querySelector("#repair");
const primaryButton = document.querySelector("#primary-action");
const settingsButton = document.querySelector("#settings");
const moreButton = document.querySelector("#more-options");
const settingsDialog = document.querySelector("#settings-dialog");
const diagnosticsToggle = document.querySelector("#diagnostics-enabled");
const DIAGNOSTICS_KEY = "rivals-diagnostics-enabled";
const statusText = document.querySelector("#client-status");
const statusKicker = document.querySelector("#status-kicker");
const detailText = document.querySelector("#progress-detail");
const speedText = document.querySelector("#transfer-speed");
const progressBar = document.querySelector("#progress");
const versionText = document.querySelector("#version");
const serviceStatus = document.querySelector("#service-status");
const serviceLabel = document.querySelector("#service-label");
const errorBanner = document.querySelector("#error-banner");
const errorMessage = document.querySelector("#error-message");
const errorDetail = document.querySelector("#error-detail");

const AUTO_CHECK_INTERVAL = 10 * 60 * 1000;
const CHECK_THROTTLE = 30 * 1000;
const NEWS_CACHE_KEY = "rivals-api-news-v1";
let newsRequest = 0;
const LANGUAGE_KEY = "rivals-launcher-language";
const languageSelect = document.querySelector("#launcher-language");
const additionalLanguageSelect = document.querySelector("#additional-language");
const addLanguageButton = document.querySelector("#add-language");
const installedLanguages = document.querySelector("#installed-languages");
const packName = (locale) => t(`language_${locale}`);
let language = chooseLanguage(localStorage.getItem(LANGUAGE_KEY), navigator.language);
const t = (key, values) => translate(language, key, values);
const GAME_LANGUAGE_KEY = "rivals-game-language";
const gameLanguageSelect = document.querySelector("#game-language");
let selectedGameLocale = localStorage.getItem(GAME_LANGUAGE_KEY);
if (!["frFR", "enUS"].includes(selectedGameLocale)) selectedGameLocale = null;
const uiLocale = () => language === "fr" ? "frFR" : "enUS";
const gameLocale = () => selectedGameLocale || uiLocale();
let availableLocales = [];
let localesKnown = false;
let installedLocales = [];
let retryLocale = null;

function applyLanguage() {
  document.documentElement.lang = language;
  languageSelect.value = language;
  gameLanguageSelect.value = gameLocale();
  document.querySelectorAll("[data-i18n]").forEach((element) => {
    element.textContent = t(element.dataset.i18n);
  });
  for (const attribute of ["aria-label", "title", "placeholder"]) {
    document.querySelectorAll(`[data-i18n-${attribute}]`).forEach((element) => {
      element.setAttribute(attribute, t(element.getAttribute(`data-i18n-${attribute}`)));
    });
  }
  document.querySelector("#hero-cta").href = `https://rivalsbeyond.com/${language}/news`;
  document.querySelector("#news-grid").replaceChildren();
  setNewsStatus("news_loading");
  renderLanguagePacks();
}

function renderLanguagePacks() {
  for (const option of gameLanguageSelect.options) {
    option.disabled = localesKnown && !availableLocales.includes(option.value);
    option.hidden = option.disabled && option.value !== gameLocale();
  }
  const remaining = availableLocales.filter((locale) => !installedLocales.includes(locale));
  const options = remaining.map((locale) => {
    const option = document.createElement("option");
    option.value = locale;
    option.textContent = packName(locale);
    return option;
  });
  if (!options.length) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = t(availableLocales.length ? "no_packs" : "packs_loading");
    options.push(option);
  }
  additionalLanguageSelect.replaceChildren(...options);
  installedLanguages.textContent = t("installed_languages", {
    languages: installedLocales.map((locale) => packName(locale)).join(", ") || t("none"),
  });
  additionalLanguageSelect.disabled = busy || checking || !remaining.length;
  addLanguageButton.disabled = busy || checking || !pathInput.value || !remaining.length;
}

const CLIENT_PATH_KEY = "moba-client-dir";
const SCROLL_BEHAVIOR = window.matchMedia("(prefers-reduced-motion: reduce)").matches
  ? "auto"
  : "smooth";

let busy = false;
let checking = false;
let currentState = { state: "install_required" };
let currentError = null;
let retryAction = "check";
let lastCheck = 0;
let progressSample = null;
let averageSpeed = 0;

function formatBytes(bytes) {
  if (!bytes) return `0 ${language === "fr" ? "o" : "B"}`;
  const units = language === "fr" ? ["o", "Ko", "Mo", "Go"] : ["B", "KB", "MB", "GB"];
  const power = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), 3);
  return `${(bytes / 1024 ** power).toFixed(power > 1 ? 1 : 0)} ${units[power]}`;
}

function formatDuration(seconds) {
  if (!Number.isFinite(seconds) || seconds < 1) return "";
  if (seconds < 60) return t("seconds_left", { seconds: Math.ceil(seconds) });
  if (seconds < 3600) return t("minutes_left", { minutes: Math.ceil(seconds / 60) });
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.ceil((seconds % 3600) / 60);
  return t("hours_left", { hours, minutes });
}

function formatDate(value) {
  const date = new Date(`${value}T00:00:00Z`);
  if (Number.isNaN(date.valueOf())) return value;
  return new Intl.DateTimeFormat(language === "fr" ? "fr-FR" : "en-US", {
    day: "numeric",
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  }).format(date);
}

function setService(kind, label) {
  serviceStatus.className = `service-status ${kind}`;
  serviceLabel.textContent = label;
}

function setBusy(value) {
  busy = value;
  chooseButton.disabled = value;
  repairButton.disabled = value || !pathInput.value;
  moreButton.disabled = value;
  settingsButton.disabled = value;
  primaryButton.disabled = value || currentState.state === "checking" || currentState.state === "game_running";
  languageSelect.disabled = value || checking;
  gameLanguageSelect.disabled = value || checking;
  renderLanguagePacks();
}

function clearError() {
  currentError = null;
  errorBanner.hidden = true;
  errorDetail.textContent = "";
}

function renderState(state) {
  currentState = state;
  if (state.available_locales) {
    availableLocales = state.available_locales;
    localesKnown = true;
  }
  if (state.installed_locales) installedLocales = state.installed_locales;
  renderLanguagePacks();
  statusKicker.textContent = t("game_client");
  progressBar.value = 0;
  speedText.textContent = "";

  const local = state.local_version;
  const remote = state.remote_version;
  switch (state.state) {
    case "checking":
      statusText.textContent = t("checking_updates");
      versionText.textContent = local ? t("version", { version: local }) : t("checking_version");
      detailText.textContent = t("connecting");
      primaryButton.textContent = t("checking");
      break;
    case "install_required":
      statusText.textContent = pathInput.value
        ? t("client_install_ready")
        : t("choose_install_folder");
      versionText.textContent = remote ? t("available_version", { version: remote }) : t("not_installed");
      detailText.textContent = t("full_install");
      primaryButton.textContent = t("install");
      break;
    case "update_available":
      statusText.textContent = t("update_available");
      versionText.textContent = local && remote
        ? t("version", { version: `${local} → ${remote}` })
        : t("version", { version: remote || t("remote") });
      detailText.textContent = t("download_ready");
      primaryButton.textContent = t("update");
      break;
    case "incomplete":
      statusText.textContent = t("interrupted");
      versionText.textContent = local ? t("version", { version: local }) : t("incomplete");
      detailText.textContent = t("resume_progress");
      primaryButton.textContent = t("resume");
      break;
    case "ready":
      statusText.textContent = t("ready_play");
      versionText.textContent = t("version", { version: remote || local || t("installed") });
      detailText.textContent = t("up_to_date");
      progressBar.value = 100;
      primaryButton.textContent = t("play");
      break;
    case "updating":
      statusKicker.textContent = t("updating_heading");
      statusText.textContent = state.message || t("preparing_update");
      versionText.textContent = state.remote_version
        ? t("install_version", { version: state.remote_version })
        : t("checking_files");
      detailText.textContent = t("preparing");
      primaryButton.textContent = t("updating");
      break;
    case "game_running":
      statusText.textContent = t("game_started");
      versionText.textContent = t("version", { version: remote || local || t("installed") });
      detailText.textContent = t("game_running");
      progressBar.value = 100;
      primaryButton.textContent = t("in_game");
      break;
    case "launcher_updating":
      statusKicker.textContent = t("launcher_updating");
      statusText.textContent = t("install_launcher", { version: state.version });
      versionText.textContent = `${state.current_version} → ${state.version}`;
      detailText.textContent = t("downloading_version");
      primaryButton.textContent = t("updating");
      break;
    case "error":
      statusKicker.textContent = t("action_required");
      statusText.textContent = localizedError(currentError);
      versionText.textContent = currentError?.code === "security"
        ? t("installation_protected")
        : t("progress_preserved");
      detailText.textContent = currentError?.retryable ? t("can_retry") : t("see_details");
      primaryButton.textContent = currentError?.retryable ? t("retry") : t("settings");
      break;
  }
  primaryButton.disabled = busy || state.state === "checking" || state.state === "game_running";
  repairButton.disabled = busy || !pathInput.value;
}

function normaliseError(error) {
  if (error && typeof error === "object" && error.message) return error;
  if (typeof error === "string") {
    try {
      const parsed = JSON.parse(error);
      if (parsed?.message) return parsed;
    } catch (_) {
      // Tauri also returns plain strings for failures outside a command.
    }
    return { code: "unknown", message: t("error_unknown"), detail: error };
  }
  return {
    code: "unknown",
    message: t("error_unknown"),
    detail: String(error),
    retryable: false,
  };
}

function localizedError(error) {
  const key = `error_${error?.code}`;
  return t(Object.hasOwn(LauncherLocales[language], key) ? key : "error_unknown");
}

function showError(error, action) {
  currentError = normaliseError(error);
  retryAction = currentError.code === "update_required" ? "update" : action;
  errorMessage.textContent = localizedError(currentError);
  errorDetail.textContent = language === "fr"
    ? currentError.detail || localizedError(currentError)
    : t("error_detail", { code: currentError.code || "unknown" });
  errorBanner.hidden = false;
  if (currentError.code === "network") {
    setService("offline", t("update_offline"));
  } else if (currentError.code === "security") {
    setService("offline", t("security_failed"));
  }
  renderState({ state: "error" });
}

function setNewsStatus(key) {
  const status = document.querySelector("#news-status");
  status.hidden = !key;
  status.textContent = key ? t(key) : "";
}

function renderNews(feed) {
  if (!Array.isArray(feed?.items) || feed.items.length > 3 || feed.items.some((item) =>
    !item || item.locale !== language || !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(item.slug)
    || typeof item.title !== "string" || typeof item.summary !== "string"
    || typeof item.publishedAt !== "string" || !Number.isFinite(Date.parse(item.publishedAt)))) return false;

  const cards = feed.items.map((item) => {
    const card = document.createElement("a");
    card.className = "news-card";
    card.href = `https://rivalsbeyond.com/${language}/news/${item.slug}`;
    card.target = "_blank";
    card.rel = "noreferrer";
    const art = document.createElement("div");
    art.className = "news-art";
    art.setAttribute("aria-hidden", "true");
    const copy = document.createElement("div");
    copy.className = "news-copy";
    const title = document.createElement("h3");
    title.textContent = item.title;
    const summary = document.createElement("p");
    summary.className = "news-summary";
    summary.textContent = item.summary;
    const time = document.createElement("time");
    time.dateTime = item.publishedAt;
    time.textContent = formatDate(item.publishedAt.slice(0, 10));
    copy.append(title, summary, time);
    card.append(art, copy);
    return card;
  });
  document.querySelector("#news-grid").replaceChildren(...cards);
  setNewsStatus(cards.length ? null : "news_empty");
  return true;
}

async function refreshNews() {
  const request = ++newsRequest;
  const requestedLanguage = language;
  const cacheKey = `${NEWS_CACHE_KEY}-${requestedLanguage}`;
  let hasCachedNews = false;
  try {
    const cached = JSON.parse(localStorage.getItem(cacheKey));
    if (cached && renderNews(cached)) hasCachedNews = cached.items.length > 0;
    else localStorage.removeItem(cacheKey);
  } catch (_) {
    localStorage.removeItem(cacheKey);
  }
  if (!invoke) {
    setNewsStatus(hasCachedNews ? "news_cached" : "news_unavailable");
    return;
  }
  try {
    const feed = await invoke("launcher_news", { locale: uiLocale() });
    if (request !== newsRequest || language !== requestedLanguage) return;
    if (!renderNews(feed)) throw new Error("Invalid news response");
    localStorage.setItem(cacheKey, JSON.stringify(feed));
  } catch (_) {
    if (request === newsRequest && language === requestedLanguage) {
      setNewsStatus(hasCachedNews ? "news_cached" : "news_unavailable");
    }
  }
}

async function refreshStatus(force = false) {
  if (!pathInput.value) {
    renderState({ state: "install_required" });
    setService("checking", t("folder_required"));
    return;
  }
  if (!invoke || busy || checking) return;
  if (!force && Date.now() - lastCheck < CHECK_THROTTLE) return;
  checking = true;
  languageSelect.disabled = true;
  gameLanguageSelect.disabled = true;
  lastCheck = Date.now();
  clearError();
  renderState({ ...currentState, state: "checking" });
  setService("checking", t("search_update"));
  try {
    let state = await invoke("client_status", { clientDir: pathInput.value, locale: gameLocale() });
    if (!selectedGameLocale && ["frFR", "enUS"].includes(state.configured_locale)) {
      const requestedLocale = gameLocale();
      selectedGameLocale = state.configured_locale;
      localStorage.setItem(GAME_LANGUAGE_KEY, selectedGameLocale);
      gameLanguageSelect.value = selectedGameLocale;
      if (requestedLocale !== selectedGameLocale) {
        state = await invoke("client_status", { clientDir: pathInput.value, locale: gameLocale() });
      }
    }
    renderState(state);
    setService("online", t("updates_online"));
  } catch (error) {
    showError(error, "check");
  } finally {
    checking = false;
    languageSelect.disabled = busy;
    gameLanguageSelect.disabled = busy;
    renderLanguagePacks();
  }
}

async function chooseDirectory() {
  if (!invoke) return;
  try {
    const selected = await invoke("choose_client_dir", { locale: uiLocale() });
    if (!selected) return;
    availableLocales = [];
    localesKnown = false;
    installedLocales = [];
    pathInput.value = selected;
    localStorage.setItem(CLIENT_PATH_KEY, selected);
    repairButton.disabled = false;
    settingsDialog.close();
    await refreshStatus(true);
  } catch (error) {
    showError(error, "check");
  }
}

async function runUpdate(repair = false, additionalLocale = null) {
  retryLocale = additionalLocale;
  if (!pathInput.value) {
    settingsDialog.showModal();
    return;
  }
  clearError();
  progressSample = null;
  averageSpeed = 0;
  renderState({
    state: "updating",
    remote_version: currentState.remote_version,
    message: repair ? t("full_verify") : t("find_files"),
  });
  setBusy(true);
  try {
    const summary = await invoke("update_client", {
      clientDir: pathInput.value,
      repair,
      locale: gameLocale(),
      additionalLocale,
    });
    if (!selectedGameLocale) {
      selectedGameLocale = gameLocale();
      localStorage.setItem(GAME_LANGUAGE_KEY, selectedGameLocale);
    }
    setService("online", t("updates_online"));
    setBusy(false);
    renderState({
      state: "ready",
      available_locales: summary.available_locales,
      installed_locales: summary.installed_locales,
      local_version: summary.version,
      remote_version: summary.version,
    });
    detailText.textContent = summary.changed_files
      ? t("files_installed", { count: summary.changed_files })
      : t("already_updated");
    speedText.textContent = t("done");
  } catch (error) {
    showError(error, "update");
  } finally {
    setBusy(false);
  }
}

async function play() {
  clearError();
  const previous = currentState;
  renderState({ ...previous, state: "game_running" });
  setBusy(true);
  try {
    await invoke("launch_game", { clientDir: pathInput.value, locale: gameLocale(), diagnosticsEnabled: diagnosticsToggle.checked });
  } catch (error) {
    showError(error, "play");
  } finally {
    setBusy(false);
  }
}

async function checkLauncherUpdate(interactive = false) {
  if (!invoke || busy) return false;
  let found = false;
  try {
    setService("checking", t("checking_launcher"));
    const update = await invoke("check_launcher_update");
    if (!update) return false;
    found = true;
    clearError();
    renderState({ state: "launcher_updating", ...update });
    setBusy(true);
    await invoke("install_launcher_update");
    return true;
  } catch (error) {
    setBusy(false);
    if (found || interactive) {
      showError(error, "launcher_update");
      return true;
    }
    return false;
  }
}

function updateLauncherProgress(payload) {
  const total = payload.total || 0;
  progressBar.value = total ? Math.min(100, payload.downloaded * 100 / total) : 0;
  detailText.textContent = total
    ? `${formatBytes(payload.downloaded)} / ${formatBytes(total)}`
    : formatBytes(payload.downloaded);
  speedText.textContent = t("auto_restart");
}

function updateProgress(payload) {
  const phase = `phase_${payload.phase}`;
  statusText.textContent = Object.hasOwn(LauncherLocales[language], phase)
    ? t(phase, { path: payload.path || "", seconds: Math.ceil(payload.delay_seconds || 0) })
    : t("preparing_update");
  const total = payload.bytes_total || payload.items_total;
  const done = payload.bytes_total ? payload.bytes_done : payload.items_done;
  progressBar.value = total ? Math.min(100, done * 100 / total) : 0;
  detailText.textContent = payload.bytes_total
    ? `${formatBytes(payload.bytes_done)} / ${formatBytes(payload.bytes_total)}`
    : `${payload.items_done} / ${payload.items_total}`;

  const downloading = payload.phase === "download" && payload.bytes_total;
  const now = performance.now();
  if (downloading && progressSample && payload.bytes_done >= progressSample.bytes) {
    const seconds = (now - progressSample.time) / 1000;
    if (seconds >= 0.35) {
      const currentSpeed = (payload.bytes_done - progressSample.bytes) / seconds;
      averageSpeed = averageSpeed ? averageSpeed * 0.7 + currentSpeed * 0.3 : currentSpeed;
      const remaining = payload.bytes_total - payload.bytes_done;
      speedText.textContent = `${formatBytes(averageSpeed)}/s · ${formatDuration(remaining / averageSpeed)}`;
      progressSample = { bytes: payload.bytes_done, time: now };
    }
  } else if (downloading) {
    progressSample = { bytes: payload.bytes_done, time: now };
  } else if (payload.phase === "retry") {
    progressSample = null;
    speedText.textContent = t("download_preserved");
  } else {
    speedText.textContent = t("local_check");
  }
}

async function primaryAction() {
  switch (currentState.state) {
    case "install_required":
      if (pathInput.value) await runUpdate(false);
      else settingsDialog.showModal();
      break;
    case "update_available":
    case "incomplete":
      await runUpdate(false);
      break;
    case "ready":
      await play();
      break;
    case "error":
      if (!currentError?.retryable) {
        settingsDialog.showModal();
      } else if (retryAction === "update") {
        await runUpdate(false, retryLocale);
      } else if (retryAction === "play") {
        await play();
      } else if (retryAction === "launcher_update") {
        await checkLauncherUpdate(true);
      } else {
        await refreshStatus(true);
      }
      break;
  }
}

function openSettings() {
  if (!busy && !settingsDialog.open) settingsDialog.showModal();
}

function scrollToSection(event) {
  const target = document.querySelector(`#${event.currentTarget.dataset.scroll}`);
  target?.scrollIntoView({ behavior: SCROLL_BEHAVIOR, block: "nearest" });
  document.querySelectorAll(".nav-link").forEach((button) => {
    button.classList.toggle("active", button.dataset.scroll === event.currentTarget.dataset.scroll);
  });
}

async function bindTauriEvents() {
  if (!listen) return;
  await listen("launcher-progress", ({ payload }) => updateProgress(payload));
  await listen("launcher-self-update-progress", ({ payload }) => {
    updateLauncherProgress(payload);
  });
  await listen("launcher-self-update-downloaded", () => {
    detailText.textContent = t("install_restart");
    speedText.textContent = t("keep_open");
  });
  await listen("game-started", () => {
    renderState({ ...currentState, state: "game_running" });
  });
  await listen("game-exited", async () => {
    setBusy(false);
    if (!await checkLauncherUpdate()) await refreshStatus(true);
  });
}

languageSelect.addEventListener("change", async () => {
  language = chooseLanguage(languageSelect.value, navigator.language);
  localStorage.setItem(LANGUAGE_KEY, language);
  applyLanguage();
  if (currentError) showError(currentError, retryAction);
  else renderState(currentState);
  refreshNews();
  await refreshStatus(true);
});
gameLanguageSelect.addEventListener("change", async () => {
  selectedGameLocale = gameLanguageSelect.value;
  localStorage.setItem(GAME_LANGUAGE_KEY, selectedGameLocale);
  await refreshStatus(true);
});
addLanguageButton.addEventListener("click", () => {
  const locale = additionalLanguageSelect.value;
  if (!availableLocales.includes(locale) || installedLocales.includes(locale)) return;
  settingsDialog.close();
  runUpdate(false, locale);
});
chooseButton.addEventListener("click", chooseDirectory);
repairButton.addEventListener("click", () => {
  settingsDialog.close();
  runUpdate(true);
});
primaryButton.addEventListener("click", primaryAction);
settingsButton.addEventListener("click", openSettings);
moreButton.addEventListener("click", openSettings);
document.querySelectorAll("[data-scroll]").forEach((button) => {
  button.addEventListener("click", scrollToSection);
});

const savedPath = localStorage.getItem(CLIENT_PATH_KEY);
diagnosticsToggle.checked = localStorage.getItem(DIAGNOSTICS_KEY) !== "false";
diagnosticsToggle.addEventListener("change", () => {
  localStorage.setItem(DIAGNOSTICS_KEY, String(diagnosticsToggle.checked));
});
if (savedPath) pathInput.value = savedPath;

async function boot() {
  if (invoke && !["fr", "en"].includes(localStorage.getItem(LANGUAGE_KEY))) {
    try {
      const osLocale = await invoke("system_locale");
      language = osLocale === "frFR" ? "fr" : "en";
    } catch (_) {
      // Browser locale remains the fallback if the native OS query is unavailable.
    }
  }
  applyLanguage();
  refreshNews();
  await bindTauriEvents();
  if (invoke) {
    renderState({ ...currentState, state: "checking" });
    statusText.textContent = t("checking_launcher");
    if (await checkLauncherUpdate()) return;
    await refreshStatus(true);
    window.setInterval(async () => {
      refreshNews();
      if (!await checkLauncherUpdate()) await refreshStatus();
    }, AUTO_CHECK_INTERVAL);
    window.addEventListener("focus", () => refreshStatus());
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) refreshStatus();
    });
  } else {
    pathInput.value = "C:\\Games\\Rivals Beyond";
    renderState({
      state: "update_available",
      local_version: "2026.8.24.1",
      remote_version: "2026.8.28.1",
      available_locales: ["frFR", "enUS"],
      installed_locales: [gameLocale()],
    });
    setService("online", t("updates_online"));
  }
}

boot();
