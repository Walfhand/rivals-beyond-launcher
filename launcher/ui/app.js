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
const NEWS_CACHE_KEY = "moba-launcher-news";
const NEWS_ARTICLE_BASE = "https://rivalsbeyond.com/fr/news/";
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
  if (!bytes) return "0 o";
  const units = ["o", "Ko", "Mo", "Go"];
  const power = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), 3);
  return `${(bytes / 1024 ** power).toFixed(power > 1 ? 1 : 0)} ${units[power]}`;
}

function formatDuration(seconds) {
  if (!Number.isFinite(seconds) || seconds < 1) return "";
  if (seconds < 60) return `${Math.ceil(seconds)} s restantes`;
  if (seconds < 3600) return `${Math.ceil(seconds / 60)} min restantes`;
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.ceil((seconds % 3600) / 60);
  return `${hours} h ${minutes} min restantes`;
}

function formatDate(value) {
  const date = new Date(`${value}T00:00:00Z`);
  if (Number.isNaN(date.valueOf())) return value;
  return new Intl.DateTimeFormat("fr-FR", {
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
  primaryButton.disabled = value || currentState.state === "checking";
}

function clearError() {
  currentError = null;
  errorBanner.hidden = true;
  errorDetail.textContent = "";
}

function renderState(state) {
  currentState = state;
  statusKicker.textContent = "CLIENT DE JEU";
  progressBar.value = 0;
  speedText.textContent = "";

  const local = state.local_version;
  const remote = state.remote_version;
  switch (state.state) {
    case "checking":
      statusText.textContent = "Recherche des mises à jour…";
      versionText.textContent = local ? `Version ${local}` : "Vérification de la version";
      detailText.textContent = "Connexion au service";
      primaryButton.textContent = "Vérification…";
      break;
    case "install_required":
      statusText.textContent = pathInput.value
        ? "Client prêt à être installé"
        : "Choisis un dossier d’installation";
      versionText.textContent = remote ? `Version disponible ${remote}` : "Version non installée";
      detailText.textContent = "Installation complète";
      primaryButton.textContent = "Installer";
      break;
    case "update_available":
      statusText.textContent = "Mise à jour disponible";
      versionText.textContent = local && remote
        ? `Version ${local} → ${remote}`
        : `Version ${remote || "distante"}`;
      detailText.textContent = "Prêt à télécharger";
      primaryButton.textContent = "Mettre à jour";
      break;
    case "incomplete":
      statusText.textContent = "Mise à jour interrompue";
      versionText.textContent = local ? `Version ${local}` : "Installation incomplète";
      detailText.textContent = "La progression sera reprise";
      primaryButton.textContent = "Reprendre";
      break;
    case "ready":
      statusText.textContent = "Prêt à jouer";
      versionText.textContent = `Version ${remote || local || "installée"}`;
      detailText.textContent = "Client à jour";
      progressBar.value = 100;
      primaryButton.textContent = "Jouer";
      break;
    case "updating":
      statusKicker.textContent = "MISE À JOUR EN COURS";
      statusText.textContent = state.message || "Préparation de la mise à jour…";
      versionText.textContent = state.remote_version
        ? `Installation de la version ${state.remote_version}`
        : "Vérification des fichiers";
      detailText.textContent = "Préparation…";
      primaryButton.textContent = "Mise à jour…";
      break;
    case "game_running":
      statusText.textContent = "Jeu lancé";
      versionText.textContent = `Version ${remote || local || "installée"}`;
      detailText.textContent = "Rivals Beyond est en cours d’exécution";
      progressBar.value = 100;
      primaryButton.textContent = "En jeu";
      break;
    case "launcher_updating":
      statusKicker.textContent = "MISE À JOUR DU LAUNCHER";
      statusText.textContent = `Installation du launcher ${state.version}`;
      versionText.textContent = `${state.current_version} → ${state.version}`;
      detailText.textContent = "Téléchargement de la nouvelle version…";
      primaryButton.textContent = "Mise à jour…";
      break;
    case "error":
      statusKicker.textContent = "ACTION REQUISE";
      statusText.textContent = currentError?.message || "Le launcher a rencontré une erreur";
      versionText.textContent = currentError?.code === "security"
        ? "Installation protégée"
        : "La progression locale est conservée";
      detailText.textContent = currentError?.retryable ? "Tu peux réessayer" : "Consulte les détails";
      primaryButton.textContent = currentError?.retryable ? "Réessayer" : "Paramètres";
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
    return { code: "unknown", message: "Le launcher a rencontré une erreur.", detail: error };
  }
  return {
    code: "unknown",
    message: "Le launcher a rencontré une erreur.",
    detail: String(error),
    retryable: false,
  };
}

function showError(error, action) {
  currentError = normaliseError(error);
  retryAction = currentError.code === "update_required" ? "update" : action;
  errorMessage.textContent = currentError.message;
  errorDetail.textContent = currentError.detail || currentError.message;
  errorBanner.hidden = false;
  if (currentError.code === "network") {
    setService("offline", "Service de mise à jour indisponible");
  } else if (currentError.code === "security") {
    setService("offline", "Vérification de sécurité échouée");
  }
  renderState({ state: "error" });
}

function renderNews(feed) {
  if (!feed?.hero || !Array.isArray(feed.items) || !feed.items.length) return;
  document.querySelector("#hero-eyebrow").textContent = feed.hero.eyebrow;
  document.querySelector("#hero-title").textContent = feed.hero.title;
  document.querySelector("#hero-summary").textContent = feed.hero.summary;
  document.querySelector("#hero-cta").textContent = feed.hero.cta;

  const items = feed.items
    .filter((item) => /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(item.slug))
    .slice(0, 3);
  if (!items.length) return;

  const cards = items.map((item) => {
    const card = document.createElement("a");
    card.className = "news-card";
    card.href = `${NEWS_ARTICLE_BASE}${item.slug}`;
    card.target = "_blank";
    card.rel = "noreferrer";
    const art = document.createElement("div");
    art.className = "news-art";
    art.setAttribute("aria-hidden", "true");
    const copy = document.createElement("div");
    copy.className = "news-copy";
    const category = document.createElement("p");
    category.className = "news-category";
    category.textContent = item.category;
    const title = document.createElement("h3");
    title.textContent = item.title;
    const summary = document.createElement("p");
    summary.className = "news-summary";
    summary.textContent = item.summary;
    const time = document.createElement("time");
    time.dateTime = item.published_at;
    time.textContent = formatDate(item.published_at);
    copy.append(category, title, summary, time);
    card.append(art, copy);
    return card;
  });
  document.querySelector("#news-grid").replaceChildren(...cards);
}

async function refreshNews() {
  try {
    const cached = JSON.parse(localStorage.getItem(NEWS_CACHE_KEY));
    renderNews(cached);
  } catch (_) {
    localStorage.removeItem(NEWS_CACHE_KEY);
  }
  if (!invoke) return;
  try {
    const feed = await invoke("launcher_news");
    renderNews(feed);
    localStorage.setItem(NEWS_CACHE_KEY, JSON.stringify(feed));
  } catch (_) {
    // Bundled or cached news remains visible when the editorial feed is offline.
  }
}

async function refreshStatus(force = false) {
  if (!pathInput.value) {
    renderState({ state: "install_required" });
    setService("checking", "Dossier d’installation à choisir");
    return;
  }
  if (!invoke || busy || checking) return;
  if (!force && Date.now() - lastCheck < CHECK_THROTTLE) return;
  checking = true;
  lastCheck = Date.now();
  clearError();
  renderState({ ...currentState, state: "checking" });
  setService("checking", "Recherche de mise à jour…");
  try {
    const state = await invoke("client_status", { clientDir: pathInput.value });
    renderState(state);
    setService("online", "Mises à jour en ligne");
  } catch (error) {
    showError(error, "check");
  } finally {
    checking = false;
  }
}

async function chooseDirectory() {
  if (!invoke) return;
  try {
    const selected = await invoke("choose_client_dir");
    if (!selected) return;
    pathInput.value = selected;
    localStorage.setItem(CLIENT_PATH_KEY, selected);
    repairButton.disabled = false;
    settingsDialog.close();
    await refreshStatus(true);
  } catch (error) {
    showError(error, "check");
  }
}

async function runUpdate(repair = false) {
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
    message: repair ? "Vérification complète du client…" : "Recherche des fichiers à installer…",
  });
  setBusy(true);
  try {
    const summary = await invoke("update_client", {
      clientDir: pathInput.value,
      repair,
    });
    setService("online", "Mises à jour en ligne");
    setBusy(false);
    renderState({
      state: "ready",
      local_version: summary.version,
      remote_version: summary.version,
    });
    detailText.textContent = summary.changed_files
      ? `${summary.changed_files} fichier(s) installé(s)`
      : "Le client est déjà à jour";
    speedText.textContent = "Terminé";
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
    await invoke("launch_game", { clientDir: pathInput.value });
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
    setService("checking", "Vérification du launcher…");
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
  speedText.textContent = "Le launcher redémarrera automatiquement";
}

function updateProgress(payload) {
  statusText.textContent = payload.message;
  const total = payload.bytes_total || payload.items_total;
  const done = payload.bytes_total ? payload.bytes_done : payload.items_done;
  progressBar.value = total ? Math.min(100, done * 100 / total) : 0;
  detailText.textContent = payload.bytes_total
    ? `${formatBytes(payload.bytes_done)} / ${formatBytes(payload.bytes_total)}`
    : `${payload.items_done} / ${payload.items_total}`;

  const downloading = payload.message.startsWith("Téléchargement") && payload.bytes_total;
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
  } else if (payload.message.startsWith("Connexion interrompue")) {
    progressSample = null;
    speedText.textContent = "La progression téléchargée est conservée";
  } else {
    speedText.textContent = "Vérification locale";
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
        await runUpdate(false);
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
    detailText.textContent = "Installation et redémarrage…";
    speedText.textContent = "Ne ferme pas le launcher";
  });
  await listen("game-started", () => {
    renderState({ ...currentState, state: "game_running" });
  });
  await listen("game-exited", async () => {
    setBusy(false);
    if (!await checkLauncherUpdate()) await refreshStatus(true);
  });
}

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
if (savedPath) pathInput.value = savedPath;

async function boot() {
  refreshNews();
  await bindTauriEvents();
  if (invoke) {
    renderState({ ...currentState, state: "checking" });
    statusText.textContent = "Vérification du launcher…";
    if (await checkLauncherUpdate()) return;
    await refreshStatus(true);
    window.setInterval(async () => {
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
    });
    setService("online", "Mises à jour en ligne");
  }
}

boot();
