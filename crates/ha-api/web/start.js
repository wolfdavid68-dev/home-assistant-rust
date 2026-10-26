// Tableau de bord — l'écran de veille de la tablette, puis le démarrage de la page.
//
// Un des fichiers du tableau de bord, chargés dans l'ordre par index.html
// (utils.js, app.js, tiles.js, dialogs.js, editor.js, account.js, start.js,
// puis canvas.js, house.js, agenda.js). Ils partagent leurs déclarations :
// une fonction d'un fichier peut appeler celle d'un autre, une fois la page
// chargée.
"use strict";

// ---------------------------------------------------------------- Écran de veille
// Après 5 minutes sans contact (en mode tablette) : le dessin de la maison en
// grand, l'heure, la météo, ce qui demande de l'attention. Plus sombre la
// nuit ; le contenu se décale un peu chaque minute, pour ne pas marquer
// l'écran. Le premier contact le fait disparaître, sans rien commander.
const SAVER_KEY = "ha-saver";
const SAVER_MS = 300_000;
const saver = { timer: 0, refresh: 0, on: false };

function saverWanted() {
  try {
    return localStorage.getItem(SAVER_KEY) !== "0";
  } catch {
    return true;
  }
}

function saverLater() {
  clearTimeout(saver.timer);
  if (!document.body.classList.contains("tablet") || !$("#saver-mode").checked) return;
  saver.timer = setTimeout(showSaver, SAVER_MS);
}

function showSaver() {
  if (saver.on || $("#dashboard-view").hidden) return;
  // Un travail en cours (éditeur, mot de passe) n'est jamais caché.
  if ($("#automations").open || $("#confirm").open) return saverLater();
  // Une vague de réveil encore en cours s'arrête net.
  wakeStop();
  saver.on = true;
  // Le dessin lui-même (avec ses effets : pluie, neige…) passe dans l'écran
  // de veille ; il retrouve sa place au réveil.
  // (Avec la vue hub, il est au centre de celle-ci : il y retournera.)
  const stage = $("#house-stage");
  const hubOn = typeof hub !== "undefined" && hub.on;
  if ((!$("#house").hidden || hubOn) && stage.childElementCount > 0) $("#saver-house").append(stage);
  // La carte du lave-linge en marche suit le dessin (house.js).
  if (typeof renderMachineLive === "function") renderMachineLive();
  // La même image, floutée, remplit tout l'écran derrière : plus de grand
  // rectangle sur fond noir, ses couleurs débordent jusqu'aux bords.
  const photo = $("#saver-house .house-base");
  const src = photo?.currentSrc || photo?.src || "";
  if (src) $("#saver").style.setProperty("--saver-photo", `url("${src}")`);
  else $("#saver").style.removeProperty("--saver-photo");
  $("#saver").hidden = false;
  renderSaver();
  saver.refresh = setInterval(renderSaver, 20_000);
}

function hideSaver(event) {
  if (!saver.on) return;
  saver.on = false;
  clearInterval(saver.refresh);
  const stage = $("#saver-house #house-stage");
  // Une copie du dessin garde l'écran de veille entier pendant la vague.
  if (stage) $("#saver-house").append(wakeGhost(stage));
  wakeWave(event);
  if (stage) {
    const hubOn = typeof hub !== "undefined" && hub.on;
    (hubOn ? $("#hub-house") : $("#house .house-frame")).append(stage);
    if (typeof house !== "undefined" && house.view) fitLabels(house.view);
    if (typeof renderMachineLive === "function") renderMachineLive();
  }
  saverLater();
}

// ---------------------------------------------------------------- Le réveil
// Étape 102 : au lieu de disparaître d'un coup, l'écran de veille s'ouvre en
// cercle depuis le doigt (ou depuis le centre, au clavier), avec une onde de
// lumière ; la maison apparaît dans le trou qui grandit. Mouvement réduit :
// un simple fondu. Pendant la vague, l'écran de veille avale encore les
// touches : rien ne s'allume sous le doigt.
const WAKE_MS = 700;
const WAKE_FADE_MS = 300;
const wake = { timer: 0, ring: null };

/** Une copie muette du dessin, sans id en double, pour la durée de la vague. */
function wakeGhost(stage) {
  const ghost = stage.cloneNode(true);
  ghost.id = "house-stage-ghost";
  ghost.setAttribute("aria-hidden", "true");
  for (const node of ghost.querySelectorAll("[id]")) node.removeAttribute("id");
  return ghost;
}

function wakeWave(event) {
  const screen = $("#saver");
  wakeStop();
  const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const width = innerWidth;
  const height = innerHeight;
  const x = Number.isFinite(event?.clientX) ? event.clientX : width / 2;
  const y = Number.isFinite(event?.clientY) ? event.clientY : height / 2;
  // Le cercle doit couvrir le coin le plus loin du doigt.
  const reach = Math.ceil(Math.hypot(Math.max(x, width - x), Math.max(y, height - y))) + 60;
  screen.style.setProperty("--wake-x", `${x}px`);
  screen.style.setProperty("--wake-y", `${y}px`);
  screen.style.setProperty("--wake-end", `${reach}px`);
  screen.classList.add(calm ? "waking-calm" : "waking");
  if (!calm) {
    wake.ring = el("span", { class: "wake-ring", "aria-hidden": "true" });
    wake.ring.style.setProperty("--wake-x", `${x}px`);
    wake.ring.style.setProperty("--wake-y", `${y}px`);
    wake.ring.style.setProperty("--wake-end", `${reach}px`);
    document.body.append(wake.ring);
  }
  wake.timer = setTimeout(() => {
    screen.hidden = true;
    wakeStop();
  }, calm ? WAKE_FADE_MS : WAKE_MS);
}

/** Fin (ou abandon) de la vague : plus de copie, plus d'onde. */
function wakeStop() {
  clearTimeout(wake.timer);
  wake.timer = 0;
  $("#saver").classList.remove("waking", "waking-calm");
  $("#house-stage-ghost")?.remove();
  wake.ring?.remove();
  wake.ring = null;
}

function renderSaver() {
  const now = new Date();
  $("#saver-clock").textContent = hourMinute.format(now);
  $("#saver-date").textContent = fullDate.format(now);
  const sun = entities.get("sun.sun");
  const night = sun ? sun.state === "below_horizon" : now.getHours() >= 22 || now.getHours() < 7;
  $("#saver").classList.toggle("night", night);
  const weather = [...entities.values()].find((s) => domainOf(s) === "weather");
  const line = $("#saver-weather");
  line.hidden = !weather;
  if (weather) {
    const label = lookup(WEATHER_LABELS, weather.state)?.[0] ?? "";
    const temp = finite(weather.attributes.temperature) ? `${number.format(weather.attributes.temperature)}°` : "";
    line.replaceChildren(svgIcon(weatherIcon(weather.state)), el("strong", { text: temp }), label);
  }
  const found = alerts();
  const status = $("#saver-status");
  status.classList.toggle("alert", found.length > 0);
  status.textContent = found.length === 0
    ? "Tout est calme."
    : found.slice(0, 3).map((a) => `${nameOf(a.state)} : ${a.text}`).join(" · ") + (found.length > 3 ? ` (et ${found.length - 3} de plus)` : "");
  const waiting = noticeStates().filter((s) => s.state === "notifying" && s.attributes.kind === "approval").length;
  $("#saver-approvals").hidden = waiting === 0;
  $("#saver-approvals").textContent = waiting === 1 ? "1 ordre attend ta validation" : `${waiting} ordres attendent ta validation`;
  // Quelques pixels de décalage, différents chaque minute.
  const minute = now.getHours() * 60 + now.getMinutes();
  $("#saver-inner").style.setProperty("--shift-x", `${(minute * 7) % 17 - 8}px`);
  $("#saver-inner").style.setProperty("--shift-y", `${(minute * 5) % 13 - 6}px`);
}

// Le premier contact réveille, et ne fait rien d'autre : il ne doit pas
// allumer la lampe qui se trouvait sous le doigt.
for (const type of ["pointerdown", "pointerup", "click"]) {
  $("#saver").addEventListener(type, (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (type === "pointerdown") hideSaver(event);
  }, { capture: true });
}
addEventListener("keydown", () => hideSaver());
$("#saver-mode").checked = saverWanted();
$("#saver-mode").addEventListener("change", (event) => {
  try {
    if (event.target.checked) localStorage.removeItem(SAVER_KEY);
    else localStorage.setItem(SAVER_KEY, "0");
  } catch {
    // Stockage bloqué : le choix ne durera que le temps de la page.
  }
  saverLater();
});

for (const type of ["pointerdown", "keydown", "scroll"]) {
  addEventListener(type, () => {
    backHomeLater();
    if (!saver.on) saverLater();
  }, { passive: true });
}
document.addEventListener("visibilitychange", keepAwake);
$("#tablet-mode").addEventListener("change", (event) => setTablet(event.target.checked, true));
setTablet(tabletWanted(), false);

// ---------------------------------------------------------------- Démarrage
function showDashboard() {
  $("#login-view").hidden = true;
  $("#dashboard-view").hidden = false;
  live.open();
  loadAccount().catch(() => {});
  loadAreas().catch(() => {});
  // Le micro (étape 84) : voix.js est chargé après ce fichier.
  if (typeof loadVoice === "function") loadVoice().catch(() => {});
  // Ouvert depuis une notification : la cloche directement.
  if (location.hash === "#notifications") {
    history.replaceState(null, "", "/");
    openFromNotification("/#notifications");
  }
}

// Rafraîchit les « il y a … » chaque minute.
// (Sauf si un curseur ou un champ de texte est en train d'être utilisé.)
setInterval(() => {
  if (entities.size && !["range", "text"].includes(document.activeElement?.type)) renderAll();
}, 60_000);

if (token.get()) showDashboard();
else showLogin();
