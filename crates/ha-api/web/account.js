// Tableau de bord — le compte, Assist, les notifications du navigateur, les intégrations, les piles, le système, le mode tablette.
//
// Un des fichiers du tableau de bord, chargés dans l'ordre par index.html
// (utils.js, app.js, tiles.js, dialogs.js, editor.js, account.js, start.js,
// puis canvas.js, house.js, agenda.js). Ils partagent leurs déclarations :
// une fonction d'un fichier peut appeler celle d'un autre, une fois la page
// chargée.
"use strict";

// ---------------------------------------------------------------- Compte
// Ce que le compte connecté peut faire (GET /api/). Le tableau de bord cache
// ce qui ne lui sert à rien ; le serveur, lui, vérifie chaque demande.
const account = { user: "", roleLabel: "", permissions: { history: true, automations: true, audit: true } };

async function loadAccount() {
  const { status, data } = await api("GET", "/api/");
  if (status !== 200) return;
  account.user = typeof data.user === "string" ? data.user : "";
  account.roleLabel = typeof data.role_label === "string" ? data.role_label : "";
  account.permissions = data.permissions ?? {};
  $("#account").replaceChildren(el("strong", { text: account.user }), account.roleLabel ? ` · ${account.roleLabel}` : "");
  // Sur une tablette murale (étape 78) : son nom, et pourquoi elle reste connectée.
  if (typeof data.wall_label === "string") {
    $("#account").append(el("br"), `Tablette murale « ${data.wall_label} » : reste connectée tant qu'elle sert.`);
  }
  $("#open-account").textContent = account.user.charAt(0);
  $("#open-logbook").hidden = !account.permissions.history;
  $("#dock-logbook").hidden = !account.permissions.history;
  $("#open-automations").hidden = !account.permissions.automations;
  $("#dock-automations").hidden = !account.permissions.automations;
  $("#open-energy").hidden = !account.permissions.history;
  $("#dock-energy").hidden = !account.permissions.history;
  $("#journal").hidden = !account.permissions.audit;
  $("#open-drawing").hidden = account.permissions.edit_dashboard !== true;
  $(".dock").style.gridTemplateColumns = `repeat(${$(".dock").querySelectorAll(":scope > :not([hidden])").length}, minmax(0, 1fr))`;
  renderAll();
  // Le service worker est (ré)enregistré à chaque ouverture : il prend la
  // version du serveur.
  loadPush().catch(() => {});
}

/** Un message bref en bas de l'écran (une commande refusée…). */
function flash(text) {
  const box = $("#flash");
  box.textContent = text.charAt(0).toUpperCase() + text.slice(1);
  box.hidden = false;
  clearTimeout(flash.timer);
  flash.timer = setTimeout(() => { box.hidden = true; }, 4000);
}

// Barre du bas (téléphone) et fenêtre du compte.
$("#dock-home").addEventListener("click", (event) => {
  event.preventDefault();
  if (view.room !== null) showRoom(null);
  window.scrollTo({ top: 0, behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
});
$("#dock-logbook").addEventListener("click", () => openLogbook());
$("#dock-automations").addEventListener("click", () => $("#open-automations").click());
$("#dock-energy").addEventListener("click", () => openEnergy());
for (const opener of ["#dock-account", "#open-account"]) {
  $(opener).addEventListener("click", () => {
    $("#account-dialog").showModal();
    watchIntegrations();
    loadPush().catch(() => {});
  });
}
$("#account-close").addEventListener("click", () => $("#account-dialog").close());

// ---------------------------------------------------------------- Assist
// « Ferme le volet de la chambre » : la phrase est comprise sur le serveur
// (étape 54), qui donne les ordres avec les mêmes vérifications qu'un
// toucher. La conversation n'est gardée que tant que la page est ouverte.

const ASSIST_KEEP = 20;

// Enchaîner les phrases (étape 96) : le conversation_id de la dernière
// réponse est renvoyé avec la phrase suivante (« et la cuisine aussi »),
// tapée ou dite, tant qu'il est récent. Le serveur l'oublie une minute après.
const ASSIST_CONVERSATION_MS = 50_000;
const assistConversation = { id: null, at: 0 };

/** La conversation en cours, si elle est encore fraîche. */
function assistConversationId() {
  const { id, at } = assistConversation;
  return id && performance.now() - at < ASSIST_CONVERSATION_MS ? id : null;
}

/** Retient la conversation de la réponse ; « merci » la termine. */
function assistRemember(data) {
  const id = data?.conversation_id;
  const more = data?.continue_conversation === true;
  if (more && typeof id === "string" && /^[0-9a-f]{32}$/.test(id)) {
    assistConversation.id = id;
    assistConversation.at = performance.now();
  } else {
    assistConversation.id = null;
  }
}

function assistSay(from, text, kind = "") {
  const log = $("#assist-log");
  log.append(el("li", { class: `assist-line from-${from}${kind ? ` ${kind}` : ""}` }, el("p", { text })));
  while (log.children.length > ASSIST_KEEP) log.firstElementChild.remove();
  log.lastElementChild.scrollIntoView({ block: "end" });
  // Une réponse en erreur : l'orbe de la voix rougit un instant (orbe.js).
  if (from === "home" && kind === "is-error" && typeof orbError === "function") orbError();
}

async function assistAsk(text) {
  const asked = text.trim();
  if (!asked) return;
  assistSay("me", asked);
  $("#assist-text").value = "";
  const submit = $("#assist-form button[type=submit]");
  submit.disabled = true;
  try {
    const conversation = assistConversationId();
    const { status, data } = await api("POST", "/api/conversation/process", {
      text: asked,
      language: "fr",
      ...(conversation ? { conversation_id: conversation } : {}),
    });
    if (status === 200) assistRemember(data);
    const response = data?.response;
    const speech = response?.speech?.plain?.speech;
    if (status === 200 && typeof speech === "string") {
      assistSay("home", speech, response.response_type === "error" ? "is-error" : "");
    } else {
      assistSay("home", typeof data?.error === "string" ? `${data.error}.` : "La maison n'a pas répondu.", "is-error");
    }
  } catch {
    assistSay("home", "La maison n'a pas répondu.", "is-error");
  } finally {
    submit.disabled = false;
    $("#assist-text").focus();
  }
}

$("#open-assist").addEventListener("click", () => {
  $("#assist").showModal();
  $("#assist-text").focus();
});
$("#assist-close").addEventListener("click", () => $("#assist").close());
$("#assist-form").addEventListener("submit", (event) => {
  event.preventDefault();
  assistAsk($("#assist-text").value);
});
for (const example of document.querySelectorAll("#assist-examples button")) {
  example.addEventListener("click", () => assistAsk(example.textContent));
}

// ---------------------------------------------------------------- Notifications du navigateur
// Dans « Mon compte » : recevoir les notifications sur ce téléphone, même
// page fermée (Web Push, étape 50). Le navigateur s'abonne avec la clé
// publique du serveur ; le service worker (sw.js) affiche ce qui arrive.

const pushView = { info: null, registration: null, subscription: null, busy: false };

/** Ce qui empêche ce navigateur de recevoir des notifications, ou null. */
function pushBlocker() {
  // Une tablette murale est partagée : pas de notifications (leurs boutons
  // valideraient au nom de celui qui les a activées).
  if (document.body.classList.contains("tablet")) {
    return "Pas de notifications en mode tablette murale : l'écran est partagé. Active-les sur ton téléphone.";
  }
  if (!window.isSecureContext) {
    return "Il faut ouvrir le tableau de bord en HTTPS (voir l'étape 7) : sans connexion sécurisée, le navigateur refuse les notifications.";
  }
  if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) {
    return /iPhone|iPad/.test(navigator.userAgent)
      ? "Sur iPhone : touche Partager, puis « Sur l'écran d'accueil », et ouvre Maison depuis l'écran d'accueil (iOS 16.4 ou plus récent)."
      : "Ce navigateur ne sait pas recevoir de notifications.";
  }
  return null;
}

/** « Pixel (Chrome) », « iPhone (Safari) »… pour reconnaître ses appareils. */
function deviceName() {
  const ua = navigator.userAgent;
  const device = /iPhone/.test(ua) ? "iPhone" : /iPad/.test(ua) ? "iPad" : /Android/.test(ua) ? "Android"
    : /Windows/.test(ua) ? "Windows" : /Macintosh/.test(ua) ? "Mac" : /Linux/.test(ua) ? "Linux" : "Appareil";
  const browser = /Edg\//.test(ua) ? "Edge" : /Firefox\//.test(ua) ? "Firefox" : /Chrome\//.test(ua) ? "Chrome"
    : /Safari\//.test(ua) ? "Safari" : "";
  return browser ? `${device} (${browser})` : device;
}

/** La clé du serveur (base64url) au format attendu par `subscribe`. */
function keyBytes(base64url) {
  const base64 = base64url.replace(/-/g, "+").replace(/_/g, "/");
  return Uint8Array.from(atob(base64 + "=".repeat((4 - (base64.length % 4)) % 4)), (c) => c.charCodeAt(0));
}

/** L'abonnement de ce navigateur, s'il y en a un. */
async function browserSubscription() {
  const registration = await navigator.serviceWorker?.getRegistration("/").catch(() => null);
  const subscription = await registration?.pushManager.getSubscription().catch(() => null);
  return { registration, subscription: subscription ?? null };
}

/** Retire ce navigateur des notifications : côté serveur (avec le jeton
 *  `bearer`, s'il est donné) et côté navigateur, et ferme celles qui sont
 *  affichées (leurs boutons ne doivent plus servir). */
async function forgetPushHere(bearer = null) {
  const { registration, subscription } = await browserSubscription();
  if (subscription && bearer) {
    const headers = { Authorization: `Bearer ${bearer}` };
    const response = await fetch("/api/push", { headers }).catch(() => null);
    const info = response?.ok ? await response.json().catch(() => null) : null;
    const mine = Array.isArray(info?.devices) ? info.devices.find((d) => d.endpoint === subscription.endpoint) : null;
    if (mine) await fetch(`/api/push/subscriptions/${mine.id}`, { method: "DELETE", headers }).catch(() => {});
  }
  await subscription?.unsubscribe().catch(() => {});
  for (const shown of (await registration?.getNotifications().catch(() => [])) ?? []) shown.close();
  pushView.subscription = null;
}

async function loadPush() {
  const section = $("#push");
  // Un invité ne reçoit pas de notifications ; sur cet appareil, celles du
  // compte précédent s'arrêtent.
  section.hidden = account.permissions.controls != null;
  if (section.hidden) {
    if ((await browserSubscription()).subscription) await forgetPushHere();
    return;
  }
  const { status, data } = await api("GET", "/api/push");
  if (status !== 200) {
    section.hidden = true;
    return;
  }
  pushView.info = data;
  // Un autre compte s'était abonné sur ce navigateur : ses notifications
  // n'ont plus rien à faire ici.
  const here = (await browserSubscription()).subscription;
  if (here && !data.devices.some((d) => d.endpoint === here.endpoint)) await forgetPushHere();
  const blocker = data.available ? pushBlocker() : "Les notifications du navigateur sont coupées sur le serveur (HA_PUSH=off).";
  if (blocker === null && pushView.registration === null) {
    try {
      pushView.registration = await navigator.serviceWorker.register("/sw.js");
      await navigator.serviceWorker.ready;
    } catch {
      pushView.registration = null;
    }
  }
  pushView.subscription = (await pushView.registration?.pushManager.getSubscription().catch(() => null)) ?? null;
  renderPush(blocker);
}

function renderPush(blocker) {
  const info = pushView.info;
  if (!info) return;
  const mine = info.devices.find((d) => d.endpoint === pushView.subscription?.endpoint) ?? null;
  const denied = "Notification" in window && Notification.permission === "denied";
  const on = mine !== null && !denied;
  let status;
  if (blocker) status = blocker;
  else if (denied) status = "Notifications bloquées pour ce site : autorise-les dans les réglages du navigateur, puis reviens ici.";
  else if (pushView.registration === null) status = "Le navigateur n'a pas pu préparer les notifications. Recharge la page.";
  else if (on) status = "Activées : cet appareil est prévenu même quand la page est fermée.";
  else status = "Reçois les ordres à valider, les alertes et les messages des automatisations, même page fermée.";
  $("#push-status").textContent = status;
  const usable = !blocker && !denied && pushView.registration !== null;
  $("#push-enable").hidden = !usable || on;
  $("#push-test").hidden = !on;
  $("#push-disable").hidden = !on;
  $("#push-categories").hidden = !on;
  // Pendant une action, les cases gardent ce que la personne vient de cocher.
  if (!pushView.busy) {
    for (const box of document.querySelectorAll("#push-categories input")) {
      box.checked = on && (mine.categories & Number(box.dataset.bit)) !== 0;
    }
  }
  for (const box of document.querySelectorAll("#push-categories input")) box.disabled = pushView.busy;
  for (const button of document.querySelectorAll("#push button")) button.disabled = pushView.busy;
  const others = info.devices.filter((d) => d !== mine);
  $("#push-devices").replaceChildren(...others.map((device) => {
    const remove = el("button", { type: "button", class: "ghost small", text: "Retirer" });
    remove.addEventListener("click", () => pushAction(async () => {
      await api("DELETE", `/api/push/subscriptions/${device.id}`);
    }));
    const seen = device.last_success ? `prévenu il y a ${ago(device.last_success)}` : "jamais prévenu";
    return el("li", { class: "push-device" },
      el("span", {}, el("strong", { text: device.name }), el("small", { text: device.failures > 0 ? `${seen} · ${device.failures} échec(s)` : seen })),
      remove);
  }));
}

/** Une action à la fois ; la section est relue après. */
async function pushAction(action) {
  if (pushView.busy) return;
  pushView.busy = true;
  renderPush(null);
  try {
    await action();
  } catch (error) {
    flash(error instanceof Error && error.message ? error.message : "Action impossible.");
  } finally {
    pushView.busy = false;
    await loadPush().catch(() => {});
  }
}

$("#push-enable").addEventListener("click", () => pushAction(async () => {
  const permission = await Notification.requestPermission();
  if (permission !== "granted") throw new Error("Notifications refusées : rien ne sera envoyé à cet appareil.");
  const manager = pushView.registration.pushManager;
  const options = { userVisibleOnly: true, applicationServerKey: keyBytes(pushView.info.public_key) };
  let subscription;
  try {
    subscription = await manager.subscribe(options);
  } catch {
    // Un ancien abonnement (autre clé du serveur) : on le remplace.
    await (await manager.getSubscription())?.unsubscribe();
    subscription = await manager.subscribe(options);
  }
  const { status, data } = await api("POST", "/api/push/subscriptions", { ...subscription.toJSON(), name: deviceName(), categories: 7 });
  if (status !== 201) {
    await subscription.unsubscribe().catch(() => {});
    throw new Error(typeof data.error === "string" ? `${data.error}.` : "Abonnement refusé par le serveur.");
  }
  flash("Notifications activées sur cet appareil.");
}));

$("#push-disable").addEventListener("click", () => pushAction(async () => {
  const mine = pushView.info.devices.find((d) => d.endpoint === pushView.subscription?.endpoint);
  if (mine) await api("DELETE", `/api/push/subscriptions/${mine.id}`);
  await pushView.subscription?.unsubscribe();
  flash("Notifications désactivées sur cet appareil.");
}));

$("#push-test").addEventListener("click", () => pushAction(async () => {
  const { status } = await api("POST", "/api/push/test");
  if (status !== 202) throw new Error("Essai impossible.");
  flash("Essai envoyé : la notification arrive dans quelques secondes.");
}));

for (const box of document.querySelectorAll("#push-categories input")) {
  box.addEventListener("change", () => {
    let categories = 0;
    for (const input of document.querySelectorAll("#push-categories input")) {
      if (input.checked) categories |= Number(input.dataset.bit);
    }
    pushAction(async () => {
      const mine = pushView.info.devices.find((d) => d.endpoint === pushView.subscription?.endpoint);
      if (mine) await api("PATCH", `/api/push/subscriptions/${mine.id}`, { categories });
    });
  });
}

/** Toucher une notification « ordre à valider » ouvre la cloche. */
function openFromNotification(url) {
  if (typeof url === "string" && url.endsWith("#notifications") && !$("#dashboard-view").hidden) {
    $("#notices").showModal();
    renderNotices();
  }
}
navigator.serviceWorker?.addEventListener("message", (event) => {
  if (event.data?.type === "open") openFromNotification(event.data.url);
});

// ---------------------------------------------------------------- Intégrations
// Dans « Mon compte » : Hue, Somfy, MQTT… connectées, ou en panne, et quand
// le serveur réessaie. Relu toutes les 10 s tant que la fenêtre est ouverte.

const INTEGRATION_LABELS = { connected: "Connectée", starting: "Démarrage…", retrying: "Ne répond pas", crashed: "Plantée, relancée", stopped: "Arrêtée" };
let integrationTimer = 0;

/** « 10:32 » aujourd'hui, « 15/10 à 10:32 » sinon. */
function shortWhen(text) {
  const date = new Date(text);
  if (Number.isNaN(date.getTime())) return "";
  const today = new Date().toDateString() === date.toDateString();
  return today ? hourMinute.format(date) : `${date.toLocaleDateString("fr", { day: "2-digit", month: "2-digit" })} à ${hourMinute.format(date)}`;
}

async function loadIntegrations() {
  const box = $("#integrations");
  if (account.permissions.all_devices === false) { box.hidden = true; return; }
  const { status, data } = await api("GET", "/api/integrations").catch(() => ({ status: 0, data: {} }));
  const list = status === 200 && Array.isArray(data.integrations) ? data.integrations : [];
  box.hidden = list.length === 0;
  $("#integration-list").replaceChildren(...list.map((item) => {
    const kind = typeof item.status === "string" ? item.status : "stopped";
    const lines = [];
    if (kind !== "stopped") lines.push(`depuis ${shortWhen(item.since)}`);
    if (item.error) lines.push(item.error);
    if (item.retry_at && kind !== "connected") lines.push(`nouvel essai à ${shortWhen(item.retry_at)}`);
    if (item.restarts > 0) lines.push(`relancée ${item.restarts} fois`);
    const plural = item.entities > 1 ? "s" : "";
    const devices = `${item.entities} appareil${plural}${item.unavailable && item.entities > 0 ? ` indisponible${plural}` : ""}`;
    return el("li", { class: `integration i-${kind}` },
      el("span", { class: "integration-dot", "aria-hidden": "true" }),
      el("span", { class: "integration-text" },
        el("b", { text: String(item.name ?? item.id) }),
        el("span", { text: `${lookup(INTEGRATION_LABELS, kind) ?? kind} ${lines.length > 0 ? "· " : ""}${lines.join(" · ")}` })),
      el("span", { class: "integration-count", text: devices }));
  }));
}

// ---------------------------------------------------------------- Piles
// Toutes les piles de la maison, la plus faible d'abord (étape 61). Les mêmes
// règles que les alertes du serveur (crates/ha-notices) : les garder pareilles.
const BATTERY_LOW = 20;
const BATTERY_EMPTY = 5;

/** Le niveau en %, `"low"` / `"ok"` pour un détecteur « pile faible », ou null. */
function batteryOf(state) {
  if (["unavailable", "unknown"].includes(state.state)) return null;
  const domain = domainOf(state);
  const isBattery = state.attributes.device_class === "battery";
  const percent = (value) => (Number.isFinite(value) && value >= 0 && value <= 100 ? value : null);
  if (domain === "sensor" && isBattery) return state.state.trim() === "" ? null : percent(Number(state.state));
  if (domain === "binary_sensor" && isBattery) return state.state === "on" ? "low" : state.state === "off" ? "ok" : null;
  if (domain === "vacuum" || domain === "persistent_notification") return null;
  return typeof state.attributes.battery_level === "number" ? percent(state.attributes.battery_level) : null;
}

function renderBatteries() {
  const box = $("#batteries");
  if (account.permissions.all_devices === false) { box.hidden = true; return; }
  // Un détecteur « pile faible » compte comme une pile à 20 % (après une
  // pile presque vide), un « pile bonne » passe après toutes les autres.
  const rank = (level) => (level === "low" ? BATTERY_LOW : level === "ok" ? 101 : level);
  const list = [...entities.values()]
    .map((state) => ({ state, level: batteryOf(state) }))
    .filter((item) => item.level !== null)
    .sort((a, b) => rank(a.level) - rank(b.level) || nameOf(a.state).localeCompare(nameOf(b.state), "fr"));
  box.hidden = list.length === 0;
  $("#battery-list").replaceChildren(...list.map(({ state, level }) => {
    const numeric = typeof level === "number";
    const kind = numeric && level <= BATTERY_EMPTY ? "crashed"
      : level === "low" || (numeric && level <= BATTERY_LOW) ? "retrying" : "connected";
    const advice = kind === "crashed" ? "à changer tout de suite" : kind === "retrying" ? "à changer bientôt" : "bonne";
    return el("li", { class: `integration i-${kind}` },
      el("span", { class: "integration-dot", "aria-hidden": "true" }),
      el("span", { class: "integration-text" },
        el("b", { text: nameOf(state) }),
        el("span", { text: level === "low" ? "Pile faible : à changer bientôt" : `Pile ${advice}` })),
      el("span", { class: "integration-count", text: numeric ? `${Math.round(level)}\u202f%` : "" }));
  }));
}

function watchIntegrations() {
  renderBatteries();
  loadIntegrations();
  loadSystem();
  loadTablets();
  clearInterval(integrationTimer);
  integrationTimer = setInterval(() => {
    if ($("#account-dialog").open) { renderBatteries(); loadIntegrations(); loadSystem(); } else clearInterval(integrationTimer);
  }, 10_000);
}

// ---------------------------------------------------------------- Tablettes murales
// Les tablettes connectées pour de bon (étape 78), et de quoi les déconnecter
// à distance (tablette volée, déménagée…) : pour l'administrateur.

/** « aujourd'hui », « hier », « il y a 12 jours » (l'usage est noté une fois par jour). */
function daysAgo(iso) {
  const days = Math.floor((Date.now() - Date.parse(iso)) / 86_400_000);
  return days <= 0 ? "aujourd'hui" : days === 1 ? "hier" : `il y a ${days} jours`;
}

async function loadTablets() {
  const box = $("#tablets");
  if (account.permissions.tablets !== true) { box.hidden = true; return; }
  const { status, data } = await api("GET", "/api/auth/tablets").catch(() => ({ status: 0, data: [] }));
  box.hidden = status !== 200;
  if (status !== 200 || !Array.isArray(data)) return;
  $("#tablet-list").replaceChildren(...data.map((tablet) => {
    const forget = el("button", { type: "button", class: "ghost small", text: "Déconnecter" });
    forget.addEventListener("click", async () => {
      if (!window.confirm(`Déconnecter « ${tablet.label} » ? Il faudra la reconnecter avec le mot de passe de son compte.`)) return;
      forget.disabled = true;
      const { status: done } = await api("DELETE", `/api/auth/tablets/${tablet.id}`).catch(() => ({ status: 0 }));
      if (done !== 204 && done !== 404) flash("Déconnexion impossible.");
      loadTablets();
    });
    return el("li", { class: "push-device" },
      el("span", {},
        el("strong", { text: tablet.label }),
        el("small", { text: `compte ${tablet.user} · active ${daysAgo(tablet.last_used_at)}` })),
      forget);
  }));
  if (data.length === 0) $("#tablet-list").append(el("li", { class: "push-device" }, el("small", { text: "Aucune tablette murale pour l'instant." })));
}

// ---------------------------------------------------------------- Système
// Ce que le serveur consomme, et son budget (étape 47) : pour l'administrateur.

const bytes = (n) => (n >= 1024 ** 2 ? `${number.format(n / 1024 ** 2)} Mio` : `${number.format(n / 1024)} Kio`);

function duration(seconds) {
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return days > 0 ? `${days} j ${hours} h` : hours > 0 ? `${hours} h ${minutes} min` : `${minutes} min`;
}

async function loadSystem() {
  const box = $("#system");
  if (account.permissions.audit === false) { box.hidden = true; return; }
  const { status, data } = await api("GET", "/api/system").catch(() => ({ status: 0, data: {} }));
  box.hidden = status !== 200;
  if (status !== 200) return;
  const budget = data.budget ?? {};
  // [libellé, valeur, dans le budget ?, budget]
  const rows = [];
  if (data.memory) {
    const mib = data.memory.resident_bytes / 1024 ** 2;
    rows.push(["Mémoire", `${bytes(data.memory.resident_bytes)} (dont ${bytes(data.memory.anonymous_bytes)} en propre)`, mib <= budget.memory_mib, `≤ ${budget.memory_mib} Mio`]);
  }
  if (typeof data.cpu_percent === "number") rows.push(["Processeur", `${number.format(data.cpu_percent)} %`, data.cpu_percent <= budget.cpu_percent, `≤ ${number.format(budget.cpu_percent)} % au repos`]);
  if (data.api) rows.push(["Réponses de l'API", `${number.format(data.api.p50_ms)} ms (95 % sous ${number.format(data.api.p95_ms)} ms)`, data.api.p95_ms <= budget.api_p95_ms, `95 % ≤ ${budget.api_p95_ms} ms`]);
  if (typeof data.database_bytes === "number") rows.push(["Base de données", bytes(data.database_bytes), true, ""]);
  rows.push(["Appareils", String(data.entities ?? 0), true, ""]);
  rows.push(["En marche depuis", duration(data.uptime_seconds ?? 0), true, `version ${data.version ?? "?"}`]);
  $("#system-list").replaceChildren(...rows.map(([label, value, ok, note]) =>
    el("div", { class: ok ? "system-row" : "system-row over" },
      el("dt", { text: label }),
      el("dd", {}, el("b", { text: value }), note ? el("span", { text: ok ? note : `au-delà du budget (${note})` }) : ""))));
  // La rapidité des appareils (étape 63) : du toucher au changement d'état,
  // intégration par intégration.
  const slowMs = Number(data.slow_ms) || 3000;
  for (const device of Array.isArray(data.devices) ? data.devices : []) {
    const late = device.p95_ms > slowMs;
    const orders = `${device.samples} ordre${device.samples > 1 ? "s" : ""}`;
    const slow = device.slow > 0 ? `, dont ${device.slow} au-delà de ${milliseconds(slowMs)}` : "";
    $("#system-list").append(el("div", { class: late ? "system-row over" : "system-row" },
      el("dt", { text: `Réponse ${device.name}` }),
      el("dd", {}, el("b", { text: `${milliseconds(device.p50_ms)} (95 % sous ${milliseconds(device.p95_ms)})` }),
        el("span", { text: `${orders}${slow}` }))));
  }
}

/** « 120 ms », « 1,8 s ». */
function milliseconds(ms) {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${number.format(Math.round(ms / 100) / 10)} s`;
}

// ---------------------------------------------------------------- Mode tablette
// Une tablette accrochée au mur : l'écran reste allumé (Wake Lock), en plein
// écran, et revient à l'accueil après 2 minutes sans contact. C'est un réglage
// de cet appareil seulement (localStorage) : rien n'est envoyé au serveur.
const TABLET_KEY = "ha-tablet";
const IDLE_MS = 120_000;
const tablet = { lock: null, idle: 0 };

function tabletWanted() {
  try {
    return localStorage.getItem(TABLET_KEY) === "1";
  } catch {
    return false; // stockage bloqué (navigation privée…)
  }
}

/** Empêche l'écran de s'éteindre. Le navigateur relâche ce verrou quand la
 *  page est cachée : on le redemande à son retour. */
async function keepAwake() {
  if (!document.body.classList.contains("tablet") || document.visibilityState !== "visible") return;
  if (tablet.lock !== null || !("wakeLock" in navigator)) return;
  try {
    tablet.lock = await navigator.wakeLock.request("screen");
    tablet.lock.addEventListener("release", () => { tablet.lock = null; });
  } catch {
    // Refusé (batterie faible, page non sécurisée…) : l'écran s'éteindra normalement.
  }
}

function setTablet(on, byHand) {
  document.body.classList.toggle("tablet", on);
  $("#tablet-mode").checked = on;
  try {
    if (on) localStorage.setItem(TABLET_KEY, "1");
    else localStorage.removeItem(TABLET_KEY);
  } catch {
    // Stockage bloqué : le mode ne durera que le temps de la page.
  }
  if (on) {
    // Écran partagé : il ne reçoit plus les notifications de personne.
    const bearer = token.get();
    forgetPushHere(bearer).then(() => (bearer ? loadPush() : undefined)).catch(() => {});
    keepAwake();
    // Le plein écran n'est permis qu'en réponse à un geste (ce clic).
    if (byHand && document.fullscreenElement === null) document.documentElement.requestFullscreen?.().catch(() => {});
  } else {
    tablet.lock?.release().catch(() => {});
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
  }
  backHomeLater();
  saverLater();
}

/** Après 2 minutes sans contact, retour à l'accueil. L'éditeur
 *  d'automatisations et la demande de mot de passe restent ouverts : on n'y
 *  perd jamais un travail en cours. */
function backHomeLater() {
  clearTimeout(tablet.idle);
  if (!document.body.classList.contains("tablet")) return;
  tablet.idle = setTimeout(() => {
    for (const dialog of document.querySelectorAll("dialog[open]")) {
      if (dialog.id !== "automations" && dialog.id !== "confirm") dialog.close();
    }
    if (view.room !== null) {
      view.room = null;
      renderAll();
    }
    window.scrollTo({ top: 0 });
    // La vue hub revient (hub.js, étape 80).
    if (typeof hubBack === "function") hubBack();
  }, IDLE_MS);
}
