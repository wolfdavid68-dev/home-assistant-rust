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
// Sur le dessin de la maison, une zone tracée sur la porte avec la sonnette
// comme appareil : elle s'allume quand on sonne (ondes, photo, heure), reste
// marquée « On a sonné » un quart d'heure, et un toucher ouvre la photo.
//
// Les images ne s'affichent pas par une simple adresse : une balise <img> ne
// sait pas envoyer le jeton. Elles sont lues avec fetch, comme le dessin de la
// maison, et jamais gardées par le navigateur (no-store).
"use strict";

const DOORBELL_OPEN = 3 * 60_000;
/** Combien de temps la porte du dessin reste marquée « On a sonné ». */
const DOORBELL_RECENT = 15 * 60_000;
const doorbell = {
  /** La sonnette qui a sonné (son identifiant), tant que la fenêtre est ouverte. */
  current: null,
  at: 0,
  closeTimer: null,
  /** Les images déjà lues : « camera.x@<image_at> » → adresse blob:. */
  urls: new Map(),
  /** L'heure du dernier coup de chaque sonnette, vu depuis l'ouverture de la page. */
  rang: new Map(),
  /** Pour retirer « On a sonné » du dessin, le moment venu. */
  zoneTimer: null,
};

function isDoorbell(state) {
  return domainOf(state) === "binary_sensor" && /_(ding|sonnette)$/.test(state.entity_id);
}

/** L'heure du coup de sonnette (en ms), si on la connaît. */
function doorbellRang(state) {
  if (state.state === "on") return Date.parse(state.last_changed) || doorbell.rang.get(state.entity_id) || Date.now();
  return doorbell.rang.get(state.entity_id) ?? null;
}

/** La zone de la sonnette sur le dessin (appelé par updateHouse). */
function doorbellZone(node, zone, state) {
  const place = zone.name ?? doorbellPlace(state);
  const ringing = state.state === "on";
  const at = doorbellRang(state);
  const recent = !ringing && at !== null && Date.now() - at < DOORBELL_RECENT;
  node.classList.add("doorbell-zone");
  node.classList.remove("readonly", "on");
  node.classList.toggle("ringing", ringing);
  node.classList.toggle("rang", recent);
  node.classList.toggle("below", zone.y < 30);
  const when = at !== null ? ` à ${hourMinute.format(new Date(at))}` : "";
  setAttr(node, "aria-label", ringing ? `On sonne : ${place}${when}. Toucher pour voir la photo.`
    : recent ? `On a sonné : ${place}${when}. Toucher pour voir la photo.` : `${place}. Toucher pour voir la dernière photo.`);
  setAttr(node, "title", place);
  if (!ringing && !recent) {
    if (node.firstChild) node.replaceChildren();
    return;
  }
  // Construit une fois : la photo se charge à part, sans tout refaire.
  let card = node.querySelector(".bell-card");
  if (!card) {
    const img = el("img", { alt: "" });
    img.hidden = true;
    card = el("span", { class: "bell-card absolute [:not(:where(.o096401))&]:[left:50%] [:not(:where(.o096402))&]:[bottom:calc(50%_+_26px)] [translate:-50%_0] grid w-[132px] overflow-hidden pointer-events-none [border-radius:12px] [background:rgb(14_14_20_/_0.88)] [color:#ffffff] text-left [box-shadow:0_10px_30px_rgb(0_0_0_/_0.5),_0_0_0_1px_rgb(255_255_255_/_0.14)] [:not(:where(.o096413))&]:[animation:bell-card-in_0.35s_ease-out_both] [:not(:where(.o096500)).house-zone.below_&]:[bottom:auto] [:not(:where(.o096501)).house-zone.below_&]:[top:calc(50%_+_26px)] [.house-zone.rang_&]:w-[max-content] [:not(:where(.o097601)).house-zone.rang_&]:[bottom:calc(50%_+_20px)] [.house-zone.rang_&]:[animation:none] [:not(:where(.o097700)).house-zone.rang.below_&]:[bottom:auto] [:not(:where(.o097701)).house-zone.rang.below_&]:[top:calc(50%_+_20px)] [@media(prefers-reduced-motion:reduce)]:[:not(:where(.o097900))&]:[animation:none]" },
      el("span", { class: "bell-photo block [aspect-ratio:4_/_3] [background:rgb(255_255_255_/_0.07)] [:not(:where(.o096700))&_img:not([hidden])]:block [&_img:not([hidden])]:w-full [&_img:not([hidden])]:h-full [&_img:not([hidden])]:[object-fit:cover] [:not(:where(.o096800)).bell-card.no-photo_&]:hidden [:not(:where(.o096800))&:has(>_img[hidden])]:hidden [:not(:where(.o097200)).house-zone.rang_&]:hidden" }, img),
      el("span", { class: "bell-text flex items-baseline justify-between gap-[8px] [padding:6px_9px_7px] [:not(:where(.o097000))&_b]:text-[0.8rem] [:not(:where(.o097001))&_b]:[color:var(--bell)] [:not(:where(.o097002))&_b]:whitespace-nowrap [:not(:where(.o097100))&_small]:text-[0.7rem] [:not(:where(.o097101))&_small]:[color:rgb(255_255_255_/_0.75)] [:not(:where(.o097102))&_small]:whitespace-nowrap [:not(:where(.o097103))&_small]:[font-variant-numeric:tabular-nums]" }, el("b"), el("small")));
    node.replaceChildren(
      el("span", { class: "bell-pin absolute [:not(:where(.o095901))&]:[left:50%] [:not(:where(.o095902))&]:[top:50%] w-[34px] h-[34px] [translate:-50%_-50%] pointer-events-none [.house-zone.rang_&]:w-[28px] [.house-zone.rang_&]:h-[28px]", "aria-hidden": "true" },
        el("i", { class: "bell-wave absolute [inset:0] [border:2px_solid_var(--bell)] [border-radius:50%] [opacity:0] [animation:bell-wave_1.8s_ease-out_infinite] [.bell-wave+&]:[animation-delay:0.9s] [.house-zone.rang_&]:hidden [@media(prefers-reduced-motion:reduce)]:hidden" }), el("i", { class: "bell-wave absolute [inset:0] [border:2px_solid_var(--bell)] [border-radius:50%] [opacity:0] [animation:bell-wave_1.8s_ease-out_infinite] [.bell-wave+&]:[animation-delay:0.9s] [.house-zone.rang_&]:hidden [@media(prefers-reduced-motion:reduce)]:hidden" }),
        el("span", { class: "bell-core absolute [inset:0] grid [place-items:center] [border-radius:50%] [background:var(--bell)] [color:#2a1a00] [box-shadow:0_4px_14px_rgb(0_0_0_/_0.45),_0_0_0_3px_rgb(255_255_255_/_0.9)] [:not(:where(.o096300))&_svg]:w-[18px] [:not(:where(.o096301))&_svg]:h-[18px] [:not(:where(.o096302))&_svg]:[stroke-width:2.3] [&_svg]:[transform-origin:50%_15%] [:not(:where(.o096304))&_svg]:[animation:bell-shake_1.2s_ease-in-out_infinite] [.house-zone.rang_&]:[box-shadow:0_3px_10px_rgb(0_0_0_/_0.4),_0_0_0_2px_rgb(255_255_255_/_0.85)] [:not(:where(.o097500)).house-zone.rang_&_svg]:w-[15px] [:not(:where(.o097501)).house-zone.rang_&_svg]:h-[15px] [.house-zone.rang_&_svg]:[animation:none] [@media(prefers-reduced-motion:reduce)]:[:not(:where(.o097900))&_svg]:[animation:none]" }, svgIcon(ICONS.bell))),
      card);
  }
  setText(card.querySelector("b"), ringing ? "On sonne" : "On a sonné");
  setText(card.querySelector("small"), when.trim());
  const camera = doorbellCamera(state.entity_id);
  const img = card.querySelector("img");
  if (camera) showCameraImage(camera, img);
  else img.hidden = true;
  card.classList.toggle("no-photo", !camera);
  if (recent) {
    clearTimeout(doorbell.zoneTimer);
    doorbell.zoneTimer = setTimeout(() => { if (house.view) updateHouse(house.view); }, at + DOORBELL_RECENT - Date.now() + 1000);
  }
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
  const response = await authFetch(`/api/camera_proxy/${encodeURIComponent(state.entity_id)}`, { cache: "no-store" });
  if (!response?.ok) return null;
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
    doorbell.rang.set(entityId, Date.parse(newState.last_changed) || Date.now());
    ringDoorbell(newState);
  }
  // La nouvelle photo arrive (quelques secondes après le coup de sonnette).
  if (doorbell.current && newState && domainOf(newState) === "camera" && $("#doorbell").open) {
    renderDoorbellPhoto();
  }
  // En v4 (hub-v4-sonnette.js), la fenêtre ouverte se met à jour elle-même.
  if (doorbell.current && newState && domainOf(newState) === "camera" && typeof hubV4 !== "undefined") hubV4.refresh?.();
}

function ringDoorbell(state) {
  // Sur un téléphone, c'est la notification qui prévient (voir docs/sonnette.md).
  if (!document.body.classList.contains("tablet")) return;
  if (typeof saver !== "undefined" && saver.on) hideSaver();
  openDoorbell(state, Date.now());
}

/** La fenêtre de la sonnette : au coup de sonnette, ou en touchant la porte
 *  sur le dessin (alors la dernière photo, sans attendre de nouvelle). */
function openDoorbell(state, at = doorbellRang(state)) {
  const live = state.state === "on";
  // Sur l'accueil v4 : la fenêtre posée à côté de la porte quand on sonne,
  // sinon la caméra en grand (hub-v4-sonnette.js).
  if (typeof hubV4Doorbell === "function" && (live ? hubV4Doorbell(state, at) : hubV4Camera(state.entity_id))) return;
  doorbell.current = state.entity_id;
  doorbell.at = live && at !== null ? at : 0;
  $("#doorbell-title").textContent = live ? `On sonne : ${doorbellPlace(state)}` : doorbellPlace(state);
  $("#doorbell-when").textContent = at === null ? "La dernière photo" : `${live ? "" : "On a sonné "}à ${hourMinute.format(new Date(at))}`;
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
