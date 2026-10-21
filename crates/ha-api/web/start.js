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
  saver.on = true;
  // Le dessin lui-même (avec ses effets : pluie, neige…) passe dans l'écran
  // de veille ; il retrouve sa place au réveil.
  // (Avec la vue hub, il est au centre de celle-ci : il y retournera.)
  const stage = $("#house-stage");
  const hubOn = typeof hub !== "undefined" && hub.on;
  if ((!$("#house").hidden || hubOn) && stage.childElementCount > 0) $("#saver-house").append(stage);
  $("#saver").hidden = false;
  renderSaver();
  saver.refresh = setInterval(renderSaver, 20_000);
}

function hideSaver() {
  if (!saver.on) return;
  saver.on = false;
  clearInterval(saver.refresh);
  $("#saver").hidden = true;
  const stage = $("#saver-house #house-stage");
  if (stage) {
    const hubOn = typeof hub !== "undefined" && hub.on;
    (hubOn ? $("#hub-house") : $("#house .house-frame")).append(stage);
    if (typeof house !== "undefined" && house.view) fitLabels(house.view);
  }
  saverLater();
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
    if (type === "pointerdown") hideSaver();
  }, { capture: true });
}
addEventListener("keydown", hideSaver);
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
