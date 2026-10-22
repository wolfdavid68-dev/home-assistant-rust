// Tableau de bord — les colis Amazon (étape 87).
//
// Le serveur lit les e-mails d'Amazon et publie `sensor.colis_amazon` : le
// nombre de colis en route, et le dernier colis à annoncer (`dernier` : en
// livraison aujourd'hui, livré, ou pas livré). La photo de l'article est
// `camera.colis_amazon`.
//
// Sur la tablette murale, comme l'Echo : une fenêtre « Plaid va arriver
// aujourd'hui. », avec la photo, par-dessus tout (écran de veille compris).
// Chaque annonce n'est montrée qu'une fois par tablette, et se ferme d'elle-même
// au bout de 30 minutes.
"use strict";

const PARCEL_SENSOR = "sensor.colis_amazon";
const PARCEL_CAMERA = "camera.colis_amazon";
const PARCEL_OPEN = 30 * 60_000;
/** Une annonce plus vieille ne s'ouvre plus (la tablette était éteinte). */
const PARCEL_FRESH = 6 * 3600_000;
const PARCEL_SEEN_KEY = "ha.colis.vus";
const parcel = { current: null, closeTimer: null, retry: null };

function parcelSeen() {
  try {
    const list = JSON.parse(localStorage.getItem(PARCEL_SEEN_KEY) ?? "[]");
    return Array.isArray(list) ? list.filter((k) => typeof k === "string") : [];
  } catch {
    return [];
  }
}

function markParcelSeen(key) {
  try {
    const list = parcelSeen().filter((k) => k !== key);
    list.push(key);
    localStorage.setItem(PARCEL_SEEN_KEY, JSON.stringify(list.slice(-50)));
  } catch {
    // Pas de stockage (navigation privée) : elle pourra se rouvrir, tant pis.
  }
}

/** Appelé à chaque changement d'état (app.js), et au chargement. */
function parcelChanged(entityId, _oldState, newState) {
  if (entityId === PARCEL_CAMERA && parcel.current && $("#parcel").open) {
    renderParcelPhoto();
    return;
  }
  if (entityId !== PARCEL_SENSOR || !newState) return;
  if (parcel.current && $("#parcel").open) renderParcel(newState);
  const last = newState.attributes.dernier;
  if (!last || typeof last !== "object" || typeof last.id !== "string") return;
  if (!["en_livraison", "livre", "probleme"].includes(last.etat)) return;
  if (!document.body.classList.contains("tablet")) return;
  const key = `${last.id}:${last.etat}`;
  if (parcelSeen().includes(key)) return;
  if (!(Date.now() - Number(last.a) < PARCEL_FRESH)) return;
  // La sonnette passe d'abord : on réessaie un peu plus tard.
  if ($("#doorbell")?.open) {
    clearTimeout(parcel.retry);
    parcel.retry = setTimeout(() => parcelChanged(PARCEL_SENSOR, null, entities.get(PARCEL_SENSOR)), 60_000);
    return;
  }
  markParcelSeen(key);
  showParcel(newState);
}

function showParcel(state) {
  if (typeof saver !== "undefined" && saver.on) hideSaver();
  parcel.current = state.attributes.dernier.id;
  renderParcel(state);
  const dialog = $("#parcel");
  if (!dialog.open) dialog.showModal();
  clearTimeout(parcel.closeTimer);
  parcel.closeTimer = setTimeout(closeParcel, PARCEL_OPEN);
}

const PARCEL_STEPS = {
  commande: "commandé",
  expedie: "en route",
  retard: "en retard",
  en_livraison: "arrive aujourd'hui",
  probleme: "pas livré",
  livre: "livré",
};

function renderParcel(state) {
  const last = state.attributes.dernier;
  if (!last) return;
  $("#parcel").classList.toggle("is-problem", last.etat === "probleme");
  $("#parcel-title").textContent = String(last.phrase ?? "");
  const at = Number(last.a);
  $("#parcel-when").textContent = Number.isFinite(at) && at > 0 ? hourMinute.format(new Date(at)) : "";
  const full = String(last.article_complet ?? "");
  $("#parcel-full").textContent = full !== last.article ? full : "";
  // Les autres colis en route (comme « Où est ma commande ? » sur l'Echo).
  const others = (Array.isArray(state.attributes.colis) ? state.attributes.colis : [])
    .filter((p) => p && p.id !== last.id && ["expedie", "retard", "en_livraison", "probleme"].includes(p.etat))
    .slice(0, 3);
  $("#parcel-others-title").hidden = others.length === 0;
  $("#parcel-others").replaceChildren(
    ...others.map((p) => el("li", {}, el("strong", { text: String(p.article ?? "") }), ` · ${PARCEL_STEPS[p.etat] ?? ""}`)),
  );
  renderParcelPhoto();
}

async function renderParcelPhoto() {
  const sensor = entities.get(PARCEL_SENSOR);
  const camera = entities.get(PARCEL_CAMERA);
  const img = $("#parcel-img");
  const icon = $("#parcel-icon");
  const ok = sensor?.attributes.dernier?.image === true && camera && camera.attributes.colis === parcel.current;
  if (!ok) {
    img.hidden = true;
    icon.hidden = false;
    return;
  }
  img.alt = String(sensor.attributes.dernier.article ?? "L'article");
  await showCameraImage(camera, img, null);
  // Pas de photo (pas encore lue, ou plus là) : l'icône d'un colis.
  icon.hidden = !img.hidden;
}

function closeParcel() {
  clearTimeout(parcel.closeTimer);
  parcel.current = null;
  if ($("#parcel").open) $("#parcel").close();
}

/** « 1 colis arrive aujourd'hui » : pour la phrase de la vue hub. */
function parcelGreeting() {
  const a = entities.get(PARCEL_SENSOR)?.attributes;
  if (!a) return null;
  const arriving = Number(a.aujourdhui) || 0;
  const delivered = Number(a.livres_aujourdhui) || 0;
  if (arriving > 0) return arriving === 1 ? "1 colis arrive aujourd'hui" : `${arriving} colis arrivent aujourd'hui`;
  if (delivered > 0) return delivered === 1 ? "1 colis livré aujourd'hui" : `${delivered} colis livrés aujourd'hui`;
  return null;
}

$("#parcel-icon").append(svgIcon(["M3 7l9-4 9 4v10l-9 4-9-4z", "M3 7l9 4 9-4", "M12 11v10", "M7.5 5l9 4"]));
$("#parcel-close").addEventListener("click", closeParcel);
$("#parcel").addEventListener("close", () => {
  parcel.current = null;
  clearTimeout(parcel.closeTimer);
});
