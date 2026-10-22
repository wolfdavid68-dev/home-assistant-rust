// Tableau de bord — la sonnette (étape 82).
//
// Une sonnette Ring (par ring-mqtt) annonce deux entités : le coup de sonnette
// (`binary_sensor.<porte>_sonnette`, « on » pendant quelques secondes) et la
// photo (`camera.<porte>_photo`, renvoyée à chaque coup de sonnette). Les
// noms anglais de ring-mqtt (`_ding`, `_snapshot`) marchent aussi.
//
// Sur la tablette murale, quand on sonne : la photo de la porte en grand,
// par-dessus tout (vue hub, écran de veille compris), qui se met à jour quand
// la nouvelle photo arrive. Elle se ferme d'elle-même au bout de 3 minutes.
//
// Les images ne s'affichent pas par une simple adresse : une balise <img> ne
// sait pas envoyer le jeton. Elles sont lues avec fetch, comme le dessin de la
// maison, et jamais gardées par le navigateur (no-store).
"use strict";

const DOORBELL_OPEN = 3 * 60_000;
const doorbell = {
  /** La sonnette qui a sonné (son identifiant), tant que la fenêtre est ouverte. */
  current: null,
  at: 0,
  closeTimer: null,
  /** Les images déjà lues : « camera.x@<image_at> » → adresse blob:. */
  urls: new Map(),
};

function isDoorbell(state) {
  return domainOf(state) === "binary_sensor" && /_(ding|sonnette)$/.test(state.entity_id);
}

/** La caméra de la même porte : `binary_sensor.porte_ding` → `camera.porte_…`. */
function doorbellCamera(entityId) {
  const base = entityId.slice("binary_sensor.".length).replace(/_(ding|sonnette)$/, "");
  const cameras = [...entities.values()]
    .filter((s) => domainOf(s) === "camera" && s.entity_id.startsWith(`camera.${base}`))
    .sort((a, b) => Number(/_(photo|snapshot)$/.test(b.entity_id)) - Number(/_(photo|snapshot)$/.test(a.entity_id)));
  return cameras[0] ?? null;
}

/** « Porte d'entrée Sonnette » → « Porte d'entrée ». */
function doorbellPlace(state) {
  return nameOf(state).replace(/\s+(ding|sonnette)$/i, "").trim() || "la porte";
}

/** L'adresse blob: de la dernière image d'une caméra (lue une fois par image). */
async function cameraUrl(state) {
  const at = Number(state.attributes.image_at);
  if (!Number.isFinite(at) || at <= 0) return null;
  const key = `${state.entity_id}@${at}`;
  if (doorbell.urls.has(key)) return doorbell.urls.get(key);
  const response = await fetch(`/api/camera_proxy/${encodeURIComponent(state.entity_id)}`, {
    headers: { Authorization: `Bearer ${token.get()}` },
    cache: "no-store",
  });
  if (!response.ok) return null;
  const url = URL.createObjectURL(await response.blob());
  for (const [old, oldUrl] of doorbell.urls) {
    if (old.startsWith(`${state.entity_id}@`)) {
      URL.revokeObjectURL(oldUrl);
      doorbell.urls.delete(old);
    }
  }
  doorbell.urls.set(key, url);
  return url;
}

/** Remplit une balise <img> avec la photo d'une caméra (et la légende). */
async function showCameraImage(state, img, caption) {
  const url = await cameraUrl(state).catch(() => null);
  if (!img.isConnected) return;
  img.hidden = !url;
  if (url && img.getAttribute("src") !== url) img.src = url;
  if (caption) {
    const at = Number(state.attributes.image_at);
    caption.textContent = Number.isFinite(at) && at > 0 ? `Photo de ${hourMinute.format(new Date(at))}` : "Pas encore de photo";
  }
}

/** Appelé à chaque changement d'état (app.js). */
function doorbellChanged(entityId, oldState, newState) {
  if (newState && isDoorbell(newState) && newState.state === "on" && oldState?.state !== "on") {
    ringDoorbell(newState);
  }
  // La nouvelle photo arrive (quelques secondes après le coup de sonnette).
  if (doorbell.current && newState && domainOf(newState) === "camera" && $("#doorbell").open) {
    renderDoorbellPhoto();
  }
}

function ringDoorbell(state) {
  // Sur un téléphone, c'est la notification qui prévient (voir docs/sonnette.md).
  if (!document.body.classList.contains("tablet")) return;
  if (typeof saver !== "undefined" && saver.on) hideSaver();
  doorbell.current = state.entity_id;
  doorbell.at = Date.now();
  $("#doorbell-title").textContent = `On sonne : ${doorbellPlace(state)}`;
  $("#doorbell-when").textContent = `à ${hourMinute.format(new Date(doorbell.at))}`;
  renderDoorbellPhoto();
  const dialog = $("#doorbell");
  if (!dialog.open) dialog.showModal();
  clearTimeout(doorbell.closeTimer);
  doorbell.closeTimer = setTimeout(closeDoorbell, DOORBELL_OPEN);
}

function renderDoorbellPhoto() {
  const camera = doorbell.current ? doorbellCamera(doorbell.current) : null;
  const img = $("#doorbell-img");
  const wait = $("#doorbell-wait");
  if (!camera) {
    img.hidden = true;
    wait.hidden = false;
    wait.textContent = "Pas de caméra pour cette sonnette.";
    $("#doorbell-shot").textContent = "";
    return;
  }
  const at = Number(camera.attributes.image_at);
  // La photo d'avant le coup de sonnette : on la montre, en attendant la nouvelle.
  const fresh = Number.isFinite(at) && at >= doorbell.at - 15_000;
  wait.hidden = fresh;
  wait.textContent = "La nouvelle photo arrive…";
  showCameraImage(camera, img, $("#doorbell-shot"));
}

function closeDoorbell() {
  clearTimeout(doorbell.closeTimer);
  doorbell.current = null;
  if ($("#doorbell").open) $("#doorbell").close();
}

$("#doorbell-close").addEventListener("click", closeDoorbell);
$("#doorbell").addEventListener("close", () => {
  doorbell.current = null;
  clearTimeout(doorbell.closeTimer);
});
