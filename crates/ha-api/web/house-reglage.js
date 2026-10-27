// La mise en page « tableau » — le réglage du dessin (administrateur) : les
// demandes pour l'IA, les images, les calques, la découpe, les valeurs, les
// zones, les courbes et les onglets.
//
// Un des fichiers de house.js, chargé après house-tableau.js par index.html.
// Les scripts partagent leurs déclarations. Pas d'innerHTML ici non plus.
"use strict";

// ---------------------------------------------------------------- Demandes pour l'IA
// Pour faire les images (le fond, les ambiances, les calques) avec une IA qui
// retouche les photos (Gemini, ChatGPT…). La méthode est celle de domo-blog :
// une image de départ, au soleil et « au repos », puis chaque variante faite
// à partir d'elle, sans rien bouger. Les demandes sont en anglais : c'est ce
// qui marche le mieux.

const AI_REFERENCE = "Use the attached image as the strict reference. Keep EXACTLY the same composition, camera angle, framing, scale, image size and the position of every object. Change ONLY what is described below. Do not add or remove anything else.";
const AI_BACKGROUND = { maquette: "Keep the plain uniform dark background (#1C1C1C), no sky, no horizon.", plan: "Keep the plain paper background, no sky, no horizon." };
/** Le temps, en mots pour l'IA. */
const AI_WEATHER = {
  "eclaircies": "PARTLY CLOUDY: bright light with a few soft cloud shadows drifting over the plot, slightly softer shadows.",
  "nuageux": "OVERCAST: soft diffuse grey light, no hard shadows, slightly muted colours.",
  "brouillard": "FOG: a light mist around the property, the far edges of the plot fading softly, diffuse light, low contrast.",
  "vent": "WINDY: trees and hedges bent by the wind, a few leaves in the air, cool grey-blue light.",
  "pluie": "RAIN: grey overcast light, visible falling rain, wet reflective ground, small puddles on the driveway and terrace.",
  "averses": "HEAVY DOWNPOUR: dark grey light, dense falling rain, streaming wet ground and roofs, larger puddles.",
  "orage": "THUNDERSTORM: very dark stormy light, heavy rain, a cold flash of lightning catching the edges of the roofs, wet reflective ground.",
  "grele": "HAIL: small white hailstones scattered on the roofs, lawn and driveway, cold grey light.",
  "neige": "SNOW: fresh snow covering the roofs, ground, hedges and trees, a few falling snowflakes, cold soft light, driveway lightly covered.",
  "pluie-neige": "SLEET: wet slushy snow patches on the roofs and lawn, rain and snow falling together, cold grey light.",
};
const AI_DUSK = "DUSK: the sun has just set, warm orange-pink glow low on the horizon, long soft shadows, blue-hour tones on the shaded walls. All interior and exterior lights stay OFF (they will be added later as separate layers).";
const AI_NIGHT = "NIGHT: deep blue ambient light, soft cool moonlight. All interior and exterior lights stay OFF (they will be added later as separate layers).";
const AI_CORRECTIONS = [
  ["camera", "Correction : le cadrage a bougé", "You moved the camera. Go back to the exact framing, camera angle and scale of the reference image, then apply only the requested change."],
  ["size", "Correction : la taille a changé", "Keep exactly the same image size and aspect ratio as the reference image."],
  ["style", "Retenir le style", "Remember this style, framing and camera angle: I will ask you for more variations of this same model, always from this reference image."],
];

/** Le nom d'image conseillé pour une ambiance (« nuit-pluie » → nuit_pluie). */
const aiFileName = (key) => key.replaceAll("-", "_");

/** Les appareils pour lesquels proposer un calque : ceux de l'onglet (boutons,
 *  zones, calques), plus les lumières et les volets de ses pièces. */
function aiLayerEntities(draft) {
  const ids = new Set([...draft.actions, ...draft.zones.map((z) => z.entity_id), ...draft.layers.map((l) => l.entity_id)]);
  const rooms = draft.areas.length > 0 ? areas.filter((a) => draft.areas.includes(a.name)) : areas;
  for (const area of rooms) for (const e of area.entities) if (/^(light|cover)\./.test(e.entity_id)) ids.add(e.entity_id);
  return [...ids].filter(Boolean).map((id) => entities.get(id)).filter((state) => state !== undefined && ["light", "cover", "switch", "lock", "valve", "fan"].includes(domainOf(state)));
}

/** « Volet · Salon » : le nom, et la pièce (il y a souvent plusieurs volets). */
function aiLayerName(state) {
  const area = areas.find((a) => a.entities.some((e) => e.entity_id === state.entity_id));
  return area && area.name !== nameOf(state) ? `${nameOf(state)} · ${area.name}` : nameOf(state);
}

/** La demande pour une image : { text, attach, save }. */
function aiPrompt(choice, draft, style) {
  const base = draft.background ?? "jour";
  const bg = AI_BACKGROUND[style];
  if (choice === "analyse") {
    return {
      attach: "1 à 3 photos de ta maison (de trois quarts, en pleine lumière)",
      save: "rien : corrige sa fiche si besoin, puis demande le fond de jour",
      text: "Look carefully at the attached photos of my house. Do NOT generate any image yet.\nWrite a numbered HOUSE SHEET: 1. overall shape and number of floors, 2. roof (shape, material, colour, chimneys, solar panels), 3. walls and colours, 4. windows on each side, 5. doors, 6. garage doors and gates, 7. plot and garden (pool, terrace, hedges, trees, driveway), 8. exterior lights.\nThen list, as questions, everything you are unsure about.",
    };
  }
  if (choice === "jour") {
    const look = style === "plan"
      ? "STYLE: clean isometric architectural blueprint drawing, fine dark-blue ink lines on white paper, light blue tints for glass and water, technical hatching on the roofs."
      : "STYLE: clean, slightly stylised 3D render, soft global illumination, premium architectural scale-model look.";
    return {
      attach: "la meilleure photo de ta maison",
      save: `${base}.webp (le fond de jour)`,
      text: [
        style === "plan" ? "Transform the attached photo of my house into an isometric architectural blueprint." : "Transform the attached photo of my house into a highly detailed 3D isometric miniature model (diorama).",
        "FIDELITY: keep the exact architecture, proportions, roof shape, number and position of windows and doors, garage, garden, pool and terrace. Do not add or remove any part of the building.",
        "CAMERA: true isometric view from a front three-quarter angle, the whole property centred with a small margin, nothing cropped.",
        look,
        "LIGHTING: bright sunny day, soft shadows. All interior and exterior lights OFF. All garage doors, gates and shutters CLOSED.",
        style === "plan" ? "BACKGROUND: plain white paper, no sky, no horizon." : "BACKGROUND: plain uniform dark background (#1C1C1C), no sky, no horizon.",
        "EXCLUDE: no text, no labels, no people, no cars, no watermark.",
        "FORMAT: landscape 16:9, highest resolution.",
      ].join("\n"),
    };
  }
  if (choice === "nuit" || AMBIANCE_KEYS.includes(choice)) {
    const moment = momentOf(choice);
    const night = moment === "nuit";
    const dusk = moment === "crepuscule";
    const weather = choice.includes("-") ? choice.slice(choice.indexOf("-") + 1) : null;
    // La nuit (ou le crépuscule) sous la pluie : à partir de la pluie de
    // jour, si on l'a déjà.
    const dayWeather = weather ? draft.ambiances[`jour-${weather}`] : null;
    const reference = moment !== "jour" && weather ? `${dayWeather ?? aiFileName(`jour-${weather}`)}.webp (la même météo de jour ; sinon ${base}.webp)` : `${base}.webp (le fond de jour)`;
    const lines = [AI_REFERENCE];
    if (night && style === "plan") lines.push("NIGHT BLUEPRINT: white ink lines on deep blue paper, like a cyanotype print. All lights stay OFF.");
    else if (night) lines.push(AI_NIGHT);
    else if (dusk && style === "plan") lines.push("DUSK BLUEPRINT: dark-blue ink lines on paper washed with a soft orange-to-blue sunset gradient. All lights stay OFF.");
    else if (dusk) lines.push(AI_DUSK);
    if (weather) lines.push(AI_WEATHER[weather]);
    if (choice === "nuit" && style !== "plan") lines.push("Clear sky feeling, calm night.");
    lines.push(bg);
    return { attach: reference, save: `${aiFileName(choice)}.webp`, text: lines.join("\n") };
  }
  if (choice.startsWith("calque:")) {
    const state = entities.get(choice.slice(7));
    if (!state) return { attach: "", save: "", text: "" };
    const name = `"${aiLayerName(state)}"`;
    const file = slug(aiLayerName(state)) || "calque";
    const domain = domainOf(state);
    const dc = state.attributes.device_class;
    let change;
    let from = `${draft.night ?? "nuit"}.webp (le fond de nuit), et refais-le avec ${base}.webp si tu veux aussi le calque de jour`;
    let save = `${file}_allume.webp`;
    if (domain === "light") {
      change = `Only turn ON the light named ${name} (location: WHERE IT IS, e.g. above the front door): warm 2700 K light with a soft halo on the nearby wall and ground.`;
    } else if (domain === "cover" && ["garage", "gate", "door"].includes(dc)) {
      change = `Only the door named ${name} (location: WHERE IT IS, e.g. the left garage door) is fully OPEN, with the dark interior visible behind it.`;
      from = `${base}.webp (le fond de jour), puis la même chose avec ${draft.night ?? "nuit"}.webp`;
      save = `${file}_ouvert.webp`;
    } else if (domain === "cover") {
      change = `Only the shutters named ${name} (location: WHERE THEY ARE, e.g. the large living-room window) are fully OPEN.`;
      from = `${base}.webp (le fond de jour)`;
      save = `${file}_ouvert.webp`;
    } else {
      change = `Only change: the device named ${name} (location: WHERE IT IS) is ON — DESCRIBE what it looks like when it is on.`;
      from = `${base}.webp (le fond de jour)`;
    }
    return {
      attach: from,
      save: `${save} — remplace d'abord « WHERE IT IS » dans la demande. Puis, dans GIMP : superpose-le au fond en mode « Différence », garde seulement ce qui change, et exporte en WebP transparent, à la taille du fond`,
      text: [AI_REFERENCE, change, "Nothing else changes: same light, same weather, same framing, same image size."].join("\n"),
    };
  }
  const correction = AI_CORRECTIONS.find(([key]) => `correction:${key}` === choice);
  return { attach: "la même conversation", save: "—", text: correction?.[2] ?? "" };
}

/** Copie un texte : le presse-papiers si la page le permet (HTTPS), sinon
 *  la sélection de la zone de texte (le réseau local en HTTP). */
async function copyText(area) {
  try {
    await navigator.clipboard.writeText(area.value);
    return true;
  } catch {
    area.focus();
    area.select();
    try {
      return document.execCommand("copy");
    } catch {
      return false;
    }
  }
}

// ---------------------------------------------------------------- Réglage
// Pour l'administrateur : les images, les fonds, les calques et les valeurs.
// Rien n'est enregistré avant « Enregistrer », sauf l'envoi d'une image.

// `pages` : la copie de tous les onglets ; `draft` : celui qu'on règle.
const drawingEdit = { pages: null, draft: null, placing: -1, tracing: -1, view: null, cut: null };

async function openDrawingEditor() {
  $("#account-dialog").close();
  await loadHouse(true);
  await Promise.all(house.images.map((image) => houseImage(image.name)));
  drawingEdit.pages = structuredClone(house.pages);
  drawingEdit.draft = drawingEdit.pages.find((page) => page.id === house.config?.id) ?? drawingEdit.pages[0];
  drawingEdit.placing = -1;
  drawingEdit.tracing = -1;
  drawingEdit.cut = null;
  drawingStatus("");
  renderDrawingEditor();
  $("#drawing").showModal();
}

function drawingStatus(text, kind = "") {
  const status = $("#drawing-status");
  status.textContent = text;
  status.className = `auto-status ${kind}`;
}

function renderDrawingPreview() {
  drawingEdit.view = drawHouse($("#drawing-preview"), drawingEdit.draft, { interactive: false });
  updateHouse(drawingEdit.view);
  drawingEdit.view.labels[drawingEdit.placing]?.node.classList.add("placing");
  drawingEdit.view.zones[drawingEdit.tracing]?.node.classList.add("placing");
  const hint = $("#drawing-place-hint");
  hint.hidden = drawingEdit.placing < 0 && drawingEdit.tracing < 0;
  hint.textContent = drawingEdit.tracing >= 0 ? "Fais glisser sur le dessin pour tracer la zone (du coin en haut à gauche au coin opposé)." : "Touche le dessin là où poser la valeur.";
  $("#drawing-preview").classList.toggle("placing", drawingEdit.placing >= 0 || drawingEdit.tracing >= 0);
}

/** En mode « placer », l'aperçu remonte en vue pour qu'on puisse le toucher. */
function showPreview() {
  if (drawingEdit.placing < 0 && drawingEdit.tracing < 0) return;
  const smooth = !matchMedia("(prefers-reduced-motion: reduce)").matches;
  $("#drawing-preview").scrollIntoView({ block: "nearest", behavior: smooth ? "smooth" : "auto" });
}

function imageOptions(selected, empty) {
  const select = el("select");
  if (empty) select.append(el("option", { value: "", text: empty }));
  for (const image of house.images) select.append(el("option", { value: image.name, text: image.name }));
  select.value = selected ?? "";
  return select;
}

function drawingFieldset(title, hint, ...children) {
  return el("fieldset", { class: "drawing-part" }, el("legend", { text: title }), hint ? el("p", { class: "auto-meta", text: hint }) : "", ...children);
}

// ---------------------------------------------------------------- Découpe
// Étape 72 : découper les calques tout seul. Deux images de la même taille,
// « tout éteint » et « allumé » (ou « volets fermés ») : ce qui change devient
// un calque par endroit, à nommer et à relier à un appareil. Tout se fait dans
// le navigateur ; seules les images finies sont envoyées.

/** Les pixels d'une image du dessin (déjà chargée). */
async function imagePixels(name) {
  const url = houseUrl(name) ?? (await houseImage(name));
  if (!url) throw new Error(`image « ${name} » introuvable`);
  const img = new Image();
  img.src = url;
  await img.decode();
  const canvas = document.createElement("canvas");
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  context.drawImage(img, 0, 0);
  return { width: canvas.width, height: canvas.height, data: context.getImageData(0, 0, canvas.width, canvas.height).data };
}

/** Ce qui change entre deux images : des régions sur une grille de
 *  `cell` pixels, regroupées à `gap` cases près. */
function findChanges(before, after, { threshold, gap, cell = 4 }) {
  const { width, height } = before;
  const diff = new Uint8Array(width * height);
  for (let i = 0, p = 0; p < diff.length; p += 1, i += 4) {
    diff[p] = Math.max(
      Math.abs(before.data[i] - after.data[i]),
      Math.abs(before.data[i + 1] - after.data[i + 1]),
      Math.abs(before.data[i + 2] - after.data[i + 2]),
    );
  }
  const gw = Math.ceil(width / cell);
  const gh = Math.ceil(height / cell);
  const marked = new Uint8Array(gw * gh);
  for (let gy = 0; gy < gh; gy += 1) {
    for (let gx = 0; gx < gw; gx += 1) {
      let hits = 0;
      let count = 0;
      for (let y = gy * cell; y < Math.min(height, (gy + 1) * cell); y += 1) {
        for (let x = gx * cell; x < Math.min(width, (gx + 1) * cell); x += 1) {
          count += 1;
          if (diff[y * width + x] > threshold) hits += 1;
        }
      }
      marked[gy * gw + gx] = hits * 4 > count ? 1 : 0;
    }
  }
  // Regrouper : on élargit, puis on cherche les morceaux d'un seul tenant.
  const grown = new Uint8Array(marked);
  for (let gy = 0; gy < gh; gy += 1) {
    for (let gx = 0; gx < gw; gx += 1) {
      if (!marked[gy * gw + gx]) continue;
      for (let dy = -gap; dy <= gap; dy += 1) {
        for (let dx = -gap; dx <= gap; dx += 1) {
          const x = gx + dx;
          const y = gy + dy;
          if (x >= 0 && y >= 0 && x < gw && y < gh) grown[y * gw + x] = 1;
        }
      }
    }
  }
  const label = new Int32Array(gw * gh).fill(-1);
  const regions = [];
  for (let start = 0; start < grown.length; start += 1) {
    if (!grown[start] || label[start] >= 0) continue;
    const id = regions.length;
    const cells = [];
    const stack = [start];
    label[start] = id;
    let minX = gw; let minY = gh; let maxX = 0; let maxY = 0; let core = 0;
    while (stack.length > 0) {
      const at = stack.pop();
      cells.push(at);
      if (marked[at]) core += 1;
      const x = at % gw;
      const y = (at - x) / gw;
      minX = Math.min(minX, x); maxX = Math.max(maxX, x);
      minY = Math.min(minY, y); maxY = Math.max(maxY, y);
      for (const next of [x > 0 ? at - 1 : -1, x < gw - 1 ? at + 1 : -1, y > 0 ? at - gw : -1, y < gh - 1 ? at + gw : -1]) {
        if (next >= 0 && grown[next] && label[next] < 0) {
          label[next] = id;
          stack.push(next);
        }
      }
    }
    regions.push({ id, cells, core, box: { x: minX * cell, y: minY * cell, w: (maxX - minX + 1) * cell, h: (maxY - minY + 1) * cell } });
  }
  // Les miettes (moins de 40 pixels changés) ne font pas un calque.
  const kept = regions.filter((region) => region.core * cell * cell >= 40 * 4).sort((a, b) => b.core - a.core).slice(0, 30);
  return { width, height, cell, gw, label, diff, regions: kept };
}

/** Le calque d'une région : les pixels de l'image « après », opaques là où
 *  ça change franchement, en fondu là où ça change peu (un halo). */
async function cutLayer(found, after, region, threshold, solid) {
  const { width, height, cell, gw, label, diff } = found;
  // Un volet est plein : on bouche les petits trous (les joints sombres des
  // lames, aussi sombres que la vitre derrière) par une fermeture de 2 px.
  const filled = solid ? closeMask(found, region, Math.max(2, threshold / 3)) : null;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  const out = context.createImageData(width, height);
  const soft = Math.max(2, threshold / 3);
  const { x: bx, y: by, w: bw, h: bh } = region.box;
  for (let y = by; y < Math.min(height, by + bh); y += 1) {
    for (let x = bx; x < Math.min(width, bx + bw); x += 1) {
      if (label[Math.floor(y / cell) * gw + Math.floor(x / cell)] !== region.id) continue;
      const p = y * width + x;
      const alpha = filled
        ? filled[(y - by) * bw + (x - bx)]
        : Math.min(1, Math.max(0, (diff[p] - soft) / (threshold * 1.5 - soft)));
      if (alpha <= 0) continue;
      const i = p * 4;
      out.data[i] = after.data[i];
      out.data[i + 1] = after.data[i + 1];
      out.data[i + 2] = after.data[i + 2];
      out.data[i + 3] = Math.round(alpha * 255);
    }
  }
  context.putImageData(out, 0, 0);
  const webp = await new Promise((resolve) => canvas.toBlob(resolve, "image/webp", 0.85));
  if (webp && webp.type === "image/webp") return webp;
  return new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
}

/** Le masque plein d'une région (dilatation puis érosion de 2 px), sur
 *  son cadre. */
function closeMask(found, region, soft) {
  const { width, cell, gw, label, diff } = found;
  const { x: bx, y: by, w: bw, h: bh } = region.box;
  let mask = new Uint8Array(bw * bh);
  for (let y = 0; y < bh; y += 1) {
    for (let x = 0; x < bw; x += 1) {
      const gx = Math.floor((bx + x) / cell);
      const gy = Math.floor((by + y) / cell);
      const p = (by + y) * width + bx + x;
      if (label[gy * gw + gx] === region.id && diff[p] > soft) mask[y * bw + x] = 1;
    }
  }
  const pass = (source, grow) => {
    const out = new Uint8Array(source.length);
    for (let y = 0; y < bh; y += 1) {
      for (let x = 0; x < bw; x += 1) {
        let any = false;
        let all = true;
        for (let dy = -2; dy <= 2; dy += 1) {
          for (let dx = -2; dx <= 2; dx += 1) {
            const xx = x + dx;
            const yy = y + dy;
            const on = xx >= 0 && yy >= 0 && xx < bw && yy < bh && source[yy * bw + xx] === 1;
            any ||= on;
            all &&= on;
          }
        }
        out[y * bw + x] = (grow ? any : all) ? 1 : 0;
      }
    }
    return out;
  };
  mask = pass(pass(mask, true), false);
  return mask;
}

/** Un aperçu réduit de l'image « après », avec les régions numérotées. */
function cutPreview(found, after) {
  const scale = Math.min(1, 720 / found.width);
  const full = document.createElement("canvas");
  full.width = found.width;
  full.height = found.height;
  full.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(after.data), found.width, found.height), 0, 0);
  const canvas = el("canvas", { class: "cut-preview" });
  canvas.width = Math.round(found.width * scale);
  canvas.height = Math.round(found.height * scale);
  const context = canvas.getContext("2d");
  context.drawImage(full, 0, 0, canvas.width, canvas.height);
  context.lineWidth = 2;
  context.font = "bold 14px sans-serif";
  found.regions.forEach((region, index) => {
    const { x, y, w, h } = region.box;
    context.strokeStyle = "#00e5ff";
    context.strokeRect(x * scale, y * scale, w * scale, h * scale);
    context.fillStyle = "#00e5ff";
    context.fillRect(x * scale, y * scale - 16, 22, 16);
    context.fillStyle = "#000";
    context.fillText(String(index + 1), x * scale + 4, y * scale - 3);
  });
  return canvas;
}

function cutPart(draft, refresh) {
  const cut = drawingEdit.cut ?? (drawingEdit.cut = { before: draft.background ?? "", after: "", threshold: 24, gap: 3, found: null, entries: [] });
  const before = imageOptions(cut.before, "— choisir —");
  before.addEventListener("change", () => { cut.before = before.value; });
  const after = imageOptions(cut.after, "— choisir —");
  after.addEventListener("change", () => { cut.after = after.value; });
  const threshold = el("input", { type: "range", min: "8", max: "80", value: String(cut.threshold), "aria-label": "Sensibilité" });
  threshold.addEventListener("change", () => { cut.threshold = Number(threshold.value); });
  const gap = el("input", { type: "range", min: "0", max: "10", value: String(cut.gap), "aria-label": "Regrouper" });
  gap.addEventListener("change", () => { cut.gap = Number(gap.value); });
  const search = el("button", { type: "button", class: "ghost small", text: "Chercher les différences" });
  search.addEventListener("click", async () => {
    if (!cut.before || !cut.after || cut.before === cut.after) return drawingStatus("Choisis deux images différentes.", "error");
    search.disabled = true;
    drawingStatus("Comparaison…");
    try {
      const [a, b] = await Promise.all([imagePixels(cut.before), imagePixels(cut.after)]);
      if (a.width !== b.width || a.height !== b.height) {
        throw new Error(`les deux images n'ont pas la même taille (${a.width}×${a.height} et ${b.width}×${b.height})`);
      }
      // Une sensibilité haute = un seuil bas.
      cut.found = findChanges(a, b, { threshold: 88 - cut.threshold, gap: cut.gap });
      cut.pixels = b;
      cut.entries = cut.found.regions.map((_, index) => ({ keep: true, name: `${cut.after}_${index + 1}`.slice(0, 40), kind: "light", entity_id: "" }));
      drawingStatus(cut.found.regions.length === 0 ? "Aucune différence trouvée : augmente la sensibilité." : `${cut.found.regions.length} endroit(s) qui changent.`, "ok");
    } catch (error) {
      drawingStatus(`Comparaison impossible : ${error.message}.`, "error");
    }
    search.disabled = false;
    refresh();
  });
  const children = [
    el("div", { class: "drawing-grid" },
      field("Avant (tout éteint, volets ouverts)", before),
      field("Après (allumé, ou volets fermés)", after),
      field("Sensibilité", threshold),
      field("Regrouper les endroits proches", gap)),
    search,
  ];
  if (cut.found && cut.found.regions.length > 0) {
    children.push(cutPreview(cut.found, cut.pixels));
    const rows = cut.entries.map((entry, index) => {
      const keep = el("input", { type: "checkbox" });
      keep.checked = entry.keep;
      keep.addEventListener("change", () => { entry.keep = keep.checked; });
      const name = el("input", { type: "text", value: entry.name, maxlength: "40", autocapitalize: "none", spellcheck: "false", "aria-label": `Nom du calque ${index + 1}` });
      name.addEventListener("change", () => { entry.name = name.value.trim(); });
      const kind = el("select", { "aria-label": `Sorte du calque ${index + 1}` },
        el("option", { value: "light", text: "Lumière allumée" }),
        el("option", { value: "cover", text: "Volet (suit sa position)" }),
        el("option", { value: "other", text: "Autre (état « on »)" }));
      kind.value = entry.kind;
      kind.addEventListener("change", () => { entry.kind = kind.value; });
      const entity = el("input", { type: "text", list: "entity-list", value: entry.entity_id, placeholder: "light.porche", autocapitalize: "none", spellcheck: "false", "aria-label": `Appareil du calque ${index + 1}` });
      entity.addEventListener("change", () => { entry.entity_id = entity.value.trim(); });
      return el("li", { class: "drawing-item" },
        el("label", { class: "check" }, keep, el("b", { text: ` Endroit ${index + 1}` })),
        el("div", { class: "drawing-grid" }, field("Nom de l'image", name), field("Sorte", kind), field("Appareil", entity)));
    });
    const create = el("button", { type: "button", class: "ghost small", text: "Créer les calques cochés" });
    create.addEventListener("click", async () => {
      const chosen = cut.entries.map((entry, index) => ({ entry, region: cut.found.regions[index] })).filter(({ entry }) => entry.keep);
      for (const { entry } of chosen) {
        if (!/^[a-z0-9_-]{1,40}$/.test(entry.name)) return drawingStatus(`Nom « ${entry.name} » : lettres minuscules, chiffres, _ et - (40 au plus).`, "error");
        if (entry.entity_id === "") return drawingStatus(`Choisis l'appareil de « ${entry.name} ».`, "error");
        if (entry.kind === "cover" && !entry.entity_id.startsWith("cover.")) return drawingStatus(`« ${entry.name} » suit un volet : un cover.… est attendu.`, "error");
      }
      create.disabled = true;
      let done = 0;
      for (const { entry, region } of chosen) {
        drawingStatus(`Découpe ${done + 1} sur ${chosen.length}…`);
        const blob = await cutLayer(cut.found, cut.pixels, region, 88 - cut.threshold, entry.kind === "cover");
        if (!blob || blob.size > MAX_HOUSE_IMAGE) {
          create.disabled = false;
          return drawingStatus(`« ${entry.name} » : image trop grosse.`, "error");
        }
        const response = await authFetch(`/api/home/images/${encodeURIComponent(entry.name)}`, {
          method: "PUT",
          headers: { "Content-Type": blob.type },
          body: blob,
        });
        if (!response?.ok) {
          const data = await response?.json().catch(() => ({}));
          create.disabled = false;
          return drawingStatus(data?.error ?? `Envoi de « ${entry.name} » impossible.`, "error");
        }
        const layer = entry.kind === "cover"
          ? { image: entry.name, entity_id: entry.entity_id, states: [], not: false, position: true, when: "always" }
          : { image: entry.name, entity_id: entry.entity_id, states: ["on"], not: false, when: "always" };
        draft.layers = draft.layers.filter((other) => other.image !== entry.name);
        draft.layers.push(layer);
        done += 1;
      }
      await loadHouse(true);
      await Promise.all(house.images.map((image) => houseImage(image.name)));
      cut.found = null;
      cut.entries = [];
      drawingStatus(`${done} calque(s) créé(s) : vérifie-les dans l'aperçu, puis « Enregistrer ».`, "ok");
      refresh();
    });
    children.push(el("ol", { class: "drawing-items" }, ...rows), create);
  }
  return drawingFieldset(
    "Découper des calques",
    "Deux images de la même taille : « avant » (tout éteint, volets ouverts) et « après » (lumières allumées, ou volets fermés). Ce qui change devient un calque par endroit : donne-lui un nom et son appareil.",
    ...children,
  );
}

function renderDrawingEditor() {
  const draft = drawingEdit.draft;
  renderDrawingPreview();
  const refresh = () => renderDrawingEditor();

  // Les images.
  const list = el("ul", { class: "drawing-images" }, ...house.images.map((image) => {
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", async () => {
      // Effacée du serveur, pas seulement du brouillon : on demande.
      if (!window.confirm(`Retirer l'image « ${image.name} » du serveur ? Les calques qui s'en servent ne l'afficheront plus.`)) return;
      const response = await authFetch(`/api/home/images/${encodeURIComponent(image.name)}`, { method: "DELETE" });
      if (!response) return drawingStatus("Serveur injoignable : l'image n'est pas retirée.", "error");
      const data = await response.json().catch(() => ({}));
      if (!response.ok) return drawingStatus(data.error ?? "Retrait impossible.", "error");
      await loadHouse(true);
      drawingStatus(`Image « ${image.name} » retirée.`, "ok");
      refresh();
    });
    const size = `${Math.max(1, Math.round(image.size / 1024))} Kio`;
    return el("li", {}, el("code", { text: image.name }), el("span", { class: "auto-meta", text: size }), remove);
  }));
  const nameInput = el("input", { type: "text", placeholder: "ex. jour, nuit, garage_ouvert", maxlength: "40", autocapitalize: "none", spellcheck: "false", "aria-label": "Nom de l'image" });
  const fileInput = el("input", { type: "file", accept: "image/png,image/jpeg,image/webp", "aria-label": "Fichier de l'image" });
  fileInput.addEventListener("change", () => {
    if (nameInput.value === "" && fileInput.files[0]) nameInput.value = slug(fileInput.files[0].name.replace(/\.[a-z0-9]+$/i, "")).replaceAll(".", "_").slice(0, 40);
  });
  const send = el("button", { type: "button", class: "ghost small", text: "Envoyer" });
  send.addEventListener("click", async () => {
    const file = fileInput.files[0];
    const name = nameInput.value.trim();
    if (!file) return drawingStatus("Choisis d'abord une image (PNG, JPEG ou WebP).", "error");
    if (!/^[a-z0-9_-]{1,40}$/.test(name)) return drawingStatus("Nom : lettres minuscules, chiffres, _ et - (40 au plus).", "error");
    if (file.size > MAX_HOUSE_IMAGE) return drawingStatus("Image trop grosse : 3 Mio au plus (exporte-la en WebP, qualité 80).", "error");
    send.disabled = true;
    drawingStatus("Envoi…");
    const response = await authFetch(`/api/home/images/${encodeURIComponent(name)}`, {
      method: "PUT",
      headers: { "Content-Type": file.type || "application/octet-stream" },
      body: file,
    });
    send.disabled = false;
    const data = await response?.json().catch(() => ({}));
    if (!response?.ok) return drawingStatus(data?.error ?? "Envoi impossible.", "error");
    await loadHouse(true);
    await houseImage(name);
    // Première image : elle devient le fond.
    if (!draft.background) draft.background = name;
    drawingStatus(`Image « ${name} » enregistrée.`, "ok");
    refresh();
  });
  const images = drawingFieldset(
    "Images",
    "PNG, JPEG ou WebP, 3 Mio au plus. Les calques doivent avoir exactement la taille du fond, avec de la transparence (PNG ou WebP).",
    list,
    el("div", { class: "drawing-row" }, nameInput, fileInput, send),
  );

  // Les demandes pour l'IA : choisir l'image à faire, copier la demande.
  const aiStyle = el("select", { "aria-label": "Style des images" },
    el("option", { value: "maquette", text: "Maquette 3D (diorama)" }), el("option", { value: "plan", text: "Plan d'architecte" }));
  aiStyle.value = drawingEdit.aiStyle ?? (isPlan() ? "plan" : "maquette");
  const aiChoice = el("select", { "aria-label": "Image à faire" },
    el("optgroup", { label: "Pour commencer" },
      el("option", { value: "analyse", text: "1. Faire décrire la maison (sans image)" }),
      el("option", { value: "jour", text: "2. Le fond de jour, au soleil" })),
    ...MOMENTS.map(([moment, momentLabel]) => el("optgroup", { label: `Ambiances : ${momentLabel.toLowerCase()}` },
      ...WEATHERS.filter(([key]) => !(moment === "jour" && key === "soleil")).map(([key, label]) => {
        const value = key === "soleil" ? moment : `${moment}-${key}`;
        const weatherText = moment !== "jour" && key === "soleil" ? "ciel clair" : label.toLowerCase();
        return el("option", { value, text: `${momentLabel}, ${weatherText}` });
      }))),
    el("optgroup", { label: "Calques" }, ...aiLayerEntities(draft).map((state) => el("option", { value: `calque:${state.entity_id}`, text: aiLayerName(state) }))),
    el("optgroup", { label: "Si l'IA se trompe" }, ...AI_CORRECTIONS.map(([key, label]) => el("option", { value: `correction:${key}`, text: label }))));
  aiChoice.value = drawingEdit.aiChoice ?? (draft.background ? "nuit" : "analyse");
  // Modifiable : on peut préciser où est le porche avant de copier.
  const aiText = el("textarea", { class: "ai-prompt", rows: "8", spellcheck: "false", "aria-label": "Demande à copier (modifiable)" });
  const aiAttach = el("span");
  const aiSave = el("span");
  const aiCopy = el("button", { type: "button", class: "primary small", text: "Copier la demande" });
  const aiShow = () => {
    drawingEdit.aiStyle = aiStyle.value;
    drawingEdit.aiChoice = aiChoice.value;
    const prompt = aiPrompt(aiChoice.value, draft, aiStyle.value);
    aiText.value = prompt.text;
    aiAttach.textContent = prompt.attach;
    aiSave.textContent = prompt.save;
  };
  aiStyle.addEventListener("change", aiShow);
  aiChoice.addEventListener("change", aiShow);
  aiCopy.addEventListener("click", async () => {
    const done = await copyText(aiText);
    drawingStatus(done ? "Demande copiée : colle-la dans Gemini ou ChatGPT, avec l'image à joindre." : "Copie impossible ici : sélectionne le texte et copie-le.", done ? "ok" : "error");
  });
  aiShow();
  const ai = drawingFieldset(
    "Faire les images avec l'IA",
    "Choisis l'image à faire, copie la demande et colle-la dans une IA qui retouche les photos (Gemini, ChatGPT…), avec l'image indiquée. Fais toujours les variantes à partir du fond de jour, jamais l'une de l'autre : le dessin ne bouge pas.",
    el("div", { class: "drawing-grid" }, field("Style", aiStyle), field("Image à faire", aiChoice)),
    aiText,
    el("dl", { class: "ai-steps" }, el("dt", { text: "Joindre" }), el("dd", {}, aiAttach), el("dt", { text: "Enregistrer sous" }), el("dd", {}, aiSave)),
    aiCopy,
  );

  // Les fonds : un par ambiance (moment × temps). Le jour au soleil est le
  // fond principal ; les autres sont facultatifs.
  const groups = [];
  for (const [moment, momentLabel] of MOMENTS) {
    const cells = [];
    for (const [weather, weatherLabel] of WEATHERS) {
      const key = weather === "soleil" ? moment : `${moment}-${weather}`;
      const main = key === "jour";
      const current = main ? draft.background : key === "nuit" ? draft.night : draft.ambiances[key];
      const select = imageOptions(current, main ? "Dessin d'exemple" : "Le plus proche");
      select.disabled = !main && !draft.background;
      select.addEventListener("change", () => {
        const value = select.value || undefined;
        if (main) {
          if (value) draft.background = value;
          else { delete draft.background; delete draft.night; draft.ambiances = {}; delete draft.forced; }
        } else if (key === "nuit") {
          if (value) draft.night = value; else delete draft.night;
        } else if (value) {
          draft.ambiances[key] = value;
        } else {
          delete draft.ambiances[key];
        }
        refresh();
      });
      cells.push(field(weatherLabel, select));
    }
    groups.push(el("div", { class: "drawing-ambiance-group" }, el("b", { text: momentLabel }), el("div", { class: "drawing-ambiances" }, ...cells)));
  }
  const forced = el("select", { "aria-label": "Ambiance affichée" },
    el("option", { value: "", text: "Automatique (météo et soleil)" }),
    ...MOMENTS.flatMap(([moment, momentLabel]) => WEATHERS.map(([weather, weatherLabel]) =>
      el("option", { value: weather === "soleil" ? moment : `${moment}-${weather}`, text: `Toujours : ${momentLabel.toLowerCase()}, ${weatherLabel.toLowerCase()}` }))));
  forced.value = draft.forced ?? "";
  forced.disabled = !draft.background;
  forced.addEventListener("change", () => {
    if (forced.value) draft.forced = forced.value; else delete draft.forced;
    renderDrawingPreview();
  });
  const nowKey = ambianceOf({ ...draft, forced: undefined });
  const nowWeather = nowKey.includes("-") ? nowKey.slice(nowKey.indexOf("-") + 1) : "soleil";
  const nowMoment = MOMENTS.find(([key]) => key === momentOf(nowKey))?.[1] ?? "Jour";
  const nowText = `Maintenant : ${nowMoment.toLowerCase()}, ${(WEATHERS.find(([key]) => key === nowWeather)?.[1] ?? nowWeather).toLowerCase()}`;
  const backgrounds = drawingFieldset(
    "Fonds et ambiances",
    "Comme sur les tableaux de domo-blog : un fond par ambiance, choisi d'après le soleil et la météo. Il en manque un ? Le plus proche s'affiche (orage → pluie → nuageux → soleil ; la nuit, puis le même temps de jour). Le crépuscule (soleil entre −6° et +6°) ne sert que si tu lui donnes au moins un fond. La pluie par 1 °C ou moins dehors compte comme de la neige.",
    ...groups,
    el("div", { class: "drawing-grid" }, field("Ambiance affichée", forced)),
    el("p", { class: "auto-meta", text: nowText }),
  );

  // Les effets animés.
  const effectMode = el("select", { "aria-label": "Effets animés" },
    el("option", { value: "", text: "Automatiques (selon la météo)" }),
    el("option", { value: "off", text: "Désactivés" }),
    ...EFFECTS.map(([key, label]) => el("option", { value: key, text: `Toujours : ${label.toLowerCase()}` })));
  effectMode.value = draft.effects ?? "";
  effectMode.addEventListener("change", () => {
    if (effectMode.value) draft.effects = effectMode.value; else delete draft.effects;
    renderDrawingPreview();
  });
  const effectLevel = el("select", { "aria-label": "Intensité des effets" }, ...EFFECT_LEVELS.map(([value, text]) => el("option", { value, text })));
  effectLevel.value = draft.effects_level ?? "";
  effectLevel.addEventListener("change", () => {
    if (effectLevel.value) draft.effects_level = effectLevel.value; else delete draft.effects_level;
    renderDrawingPreview();
  });
  const nowEffect = effectOf({ ...draft, effects: undefined });
  const effects = drawingFieldset(
    "Effets animés",
    "Sur le dessin, le temps qu'il fait : pluie, averses, orage, grêle, neige, brouillard, vent, et des étoiles par nuit claire. Ils suivent la météo comme les ambiances ; « Toujours » sert à essayer. Pas d'animation si l'appareil demande de réduire les mouvements.",
    el("div", { class: "drawing-grid" }, field("Effets", effectMode), field("Intensité", effectLevel)),
    el("p", { class: "auto-meta", text: `Maintenant : ${nowEffect ? (EFFECTS.find(([key]) => key === nowEffect)?.[1] ?? nowEffect).toLowerCase() : "aucun effet"}` }),
  );

  // Les calques.
  const layerRows = draft.layers.map((layer, index) => {
    const image = imageOptions(layer.image, null);
    image.addEventListener("change", () => { layer.image = image.value; renderDrawingPreview(); });
    // Le calque suit un appareil, ou s'allume avec les lampes d'une pièce.
    const kind = el("select", {}, el("option", { value: "entity", text: "Un appareil" }), el("option", { value: "area", text: "Une pièce (dès qu'une lampe y est allumée)" }));
    kind.value = layer.area !== undefined ? "area" : "entity";
    kind.disabled = areas.length === 0 && layer.area === undefined;
    kind.addEventListener("change", () => {
      if (kind.value === "area") {
        delete layer.entity_id;
        Object.assign(layer, { area: areas[0]?.name ?? "", states: [], not: false, position: false });
      } else {
        delete layer.area;
        Object.assign(layer, { entity_id: "", states: ["on"] });
      }
      refresh();
    });
    const when = el("select", {}, ...WHEN_LABELS.map(([value, text]) => el("option", { value, text })));
    when.value = layer.when;
    when.addEventListener("change", () => { layer.when = when.value; renderDrawingPreview(); });
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", () => { draft.layers.splice(index, 1); refresh(); });
    const shown = layerShown(layer, houseNight());
    const head = el("div", { class: "drawing-item-head" }, el("b", { text: `Calque ${index + 1}` }),
      el("span", { class: shown ? "badge ok" : "badge", text: shown ? "visible maintenant" : "caché maintenant" }), remove);
    if (layer.area !== undefined) {
      const room = el("select", {}, ...areas.map((area) => el("option", { value: area.name, text: area.name })));
      room.value = layer.area;
      room.addEventListener("change", () => { layer.area = room.value; renderDrawingPreview(); });
      return el("li", { class: "drawing-item" }, head,
        el("div", { class: "drawing-grid" },
          field("Image", image), field("Suit", kind), field("Pièce", room), field("Moment", when)));
    }
    const entity = el("input", { type: "text", list: "entity-list", value: layer.entity_id, placeholder: "cover.garage", autocapitalize: "none", spellcheck: "false" });
    entity.addEventListener("change", () => { layer.entity_id = entity.value.trim(); renderDrawingPreview(); });
    const states = el("input", { type: "text", value: layer.states.join(", "), placeholder: "open, opening" });
    states.addEventListener("change", () => {
      layer.states = states.value.split(",").map((s) => s.trim()).filter(Boolean);
      renderDrawingPreview();
    });
    const not = el("input", { type: "checkbox" });
    not.checked = layer.not;
    not.addEventListener("change", () => { layer.not = not.checked; renderDrawingPreview(); });
    // Un volet qui suit sa position : l'image est le volet fermé.
    const position = el("input", { type: "checkbox" });
    position.checked = layer.position === true;
    states.disabled = position.checked;
    not.disabled = position.checked;
    position.addEventListener("change", () => {
      layer.position = position.checked;
      states.disabled = position.checked;
      not.disabled = position.checked;
      renderDrawingPreview();
    });
    return el("li", { class: "drawing-item" }, head,
      el("div", { class: "drawing-grid" },
        field("Image", image), field("Suit", kind), field("Appareil", entity),
        field("Quand son état est (séparés par des virgules)", states), field("Moment", when)),
      el("label", { class: "check" }, not, " Sauf ces états (visible pour tous les autres)"),
      el("label", { class: "check" }, position, " Suit la position du volet (l'image est le volet fermé ; à 25 %, il est dessiné aux trois quarts)"));
  });
  const addLayer = el("button", { type: "button", class: "ghost small", text: "+ Ajouter un calque" });
  addLayer.disabled = house.images.length === 0;
  addLayer.addEventListener("click", () => {
    draft.layers.push({ image: house.images.at(-1)?.name ?? "", entity_id: "", states: ["on"], not: false, when: "always" });
    refresh();
  });
  const layers = drawingFieldset(
    "Calques",
    "Ex. : l'image garage_ouvert quand cover.garage est open ou opening ; porche_allume quand light.porche est on, la nuit ; fenetre_cuisine quand une lampe de la Cuisine est allumée.",
    el("ol", { class: "drawing-items" }, ...layerRows),
    addLayer,
  );

  // Les valeurs posées sur le dessin.
  const labelRows = draft.labels.map((label, index) => {
    const entity = el("input", { type: "text", list: "entity-list", value: label.entity_id, placeholder: "sensor.temperature_salon", autocapitalize: "none", spellcheck: "false" });
    entity.addEventListener("change", () => { label.entity_id = entity.value.trim(); renderDrawingPreview(); });
    const name = el("input", { type: "text", value: label.name ?? "", placeholder: "(le nom de l'appareil)", maxlength: "60" });
    name.addEventListener("change", () => {
      if (name.value.trim()) label.name = name.value.trim();
      else delete label.name;
      renderDrawingPreview();
    });
    const place = el("button", { type: "button", class: drawingEdit.placing === index ? "primary small" : "ghost small", text: drawingEdit.placing === index ? "Touche le dessin…" : `Placer (${number.format(label.x)} %, ${number.format(label.y)} %)` });
    place.addEventListener("click", () => {
      drawingEdit.placing = drawingEdit.placing === index ? -1 : index;
      drawingEdit.tracing = -1;
      refresh();
      showPreview();
    });
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", () => {
      draft.labels.splice(index, 1);
      drawingEdit.placing = -1;
      refresh();
    });
    return el("li", { class: "drawing-item" },
      el("div", { class: "drawing-item-head" }, el("b", { text: `Valeur ${index + 1}` }), remove),
      el("div", { class: "drawing-grid" }, field("Appareil", entity), field("Nom affiché", name)),
      place);
  });
  const addLabel = el("button", { type: "button", class: "ghost small", text: "+ Ajouter une valeur" });
  addLabel.addEventListener("click", () => {
    draft.labels.push({ entity_id: "", x: 50, y: 50 });
    drawingEdit.placing = -1;
    refresh();
    // On choisit d'abord l'appareil ; « Placer » viendra ensuite.
    $("#drawing-form fieldset:last-of-type .drawing-item:last-child input")?.focus();
  });
  const labels = drawingFieldset(
    "Valeurs sur le dessin",
    "Une température, une puissance, un état… posé où tu veux : « Placer », puis touche le dessin.",
    el("ol", { class: "drawing-items" }, ...labelRows),
    addLabel,
  );

  // Les boutons rapides sous le dessin.
  const actionRows = draft.actions.map((id, index) => {
    const entity = el("input", { type: "text", list: "entity-list", value: id, placeholder: "light.porche", autocapitalize: "none", spellcheck: "false", "aria-label": `Bouton ${index + 1}` });
    entity.addEventListener("change", () => { draft.actions[index] = entity.value.trim(); });
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", () => { draft.actions.splice(index, 1); refresh(); });
    return el("li", { class: "drawing-row" }, entity, remove);
  });
  const addAction = el("button", { type: "button", class: "ghost small", text: "+ Ajouter un bouton" });
  addAction.disabled = draft.actions.length >= 12;
  addAction.addEventListener("click", () => {
    draft.actions.push("");
    refresh();
    $("#drawing-form fieldset:last-of-type li:last-child input")?.focus();
  });
  const actions = drawingFieldset(
    "Boutons sous le dessin",
    "Le porche, le parking, les portes de garage… : un toucher allume, éteint, ouvre ou ferme (12 au plus).",
    el("ol", { class: "drawing-items" }, ...actionRows),
    addAction,
  );

  // Les zones à toucher sur le dessin.
  const zoneRows = draft.zones.map((zone, index) => {
    // Une zone commande un appareil, ou ouvre une pièce.
    const kind = el("select", {}, el("option", { value: "entity", text: "Commander un appareil" }), el("option", { value: "area", text: "Ouvrir une pièce" }));
    kind.value = zone.area !== undefined ? "area" : "entity";
    kind.disabled = areas.length === 0 && zone.area === undefined;
    kind.addEventListener("change", () => {
      if (kind.value === "area") { delete zone.entity_id; zone.area = areas[0]?.name ?? ""; } else { delete zone.area; zone.entity_id = ""; }
      refresh();
    });
    let target;
    if (zone.area !== undefined) {
      target = el("select", {}, ...areas.map((area) => el("option", { value: area.name, text: area.name })));
      target.value = zone.area;
      target.addEventListener("change", () => { zone.area = target.value; });
    } else {
      target = el("input", { type: "text", list: "entity-list", value: zone.entity_id ?? "", placeholder: "light.porche", autocapitalize: "none", spellcheck: "false" });
      target.addEventListener("change", () => { zone.entity_id = target.value.trim(); });
    }
    const name = el("input", { type: "text", value: zone.name ?? "", placeholder: zone.area !== undefined ? "(le nom de la pièce)" : "(le nom de l'appareil)", maxlength: "60" });
    name.addEventListener("change", () => { if (name.value.trim()) zone.name = name.value.trim(); else delete zone.name; });
    const tracing = drawingEdit.tracing === index;
    const trace = el("button", { type: "button", class: tracing ? "primary small" : "ghost small", text: tracing ? "Trace sur le dessin…" : `Tracer (${number.format(zone.w)} × ${number.format(zone.h)} %)` });
    trace.addEventListener("click", () => {
      drawingEdit.tracing = tracing ? -1 : index;
      drawingEdit.placing = -1;
      refresh();
      showPreview();
    });
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", () => {
      draft.zones.splice(index, 1);
      drawingEdit.tracing = -1;
      refresh();
    });
    return el("li", { class: "drawing-item" },
      el("div", { class: "drawing-item-head" }, el("b", { text: `Zone ${index + 1}` }), remove),
      el("div", { class: "drawing-grid" }, field("Un toucher…", kind), field(zone.area !== undefined ? "Pièce" : "Appareil", target), field("Nom affiché", name)),
      trace);
  });
  const addZone = el("button", { type: "button", class: "ghost small", text: "+ Ajouter une zone" });
  addZone.disabled = draft.zones.length >= 24;
  addZone.addEventListener("click", () => {
    draft.zones.push({ entity_id: "", x: 40, y: 40, w: 20, h: 20 });
    drawingEdit.tracing = -1;
    drawingEdit.placing = -1;
    refresh();
    $("#drawing-form .drawing-zones .drawing-item:last-child input")?.focus();
  });
  // La sonnette pas encore posée sur le dessin : un bouton pour tracer sa porte.
  const bell = [...entities.values()].find(isDoorbell);
  let addBell = "";
  if (bell && !draft.zones.some((z) => z.entity_id === bell.entity_id)) {
    addBell = el("button", { type: "button", class: "ghost small", text: "+ Placer la sonnette" });
    addBell.disabled = draft.zones.length >= 24;
    addBell.addEventListener("click", () => {
      draft.zones.push({ entity_id: bell.entity_id, name: doorbellPlace(bell), x: 45, y: 50, w: 8, h: 14 });
      drawingEdit.tracing = draft.zones.length - 1;
      drawingEdit.placing = -1;
      refresh();
      showPreview();
    });
  }
  const zones = drawingFieldset(
    "Zones à toucher",
    "Le porche, la porte du garage, le salon… : trace un rectangle sur le dessin. Un toucher allume, éteint, ouvre ou ferme l'appareil, ou ouvre la pièce ; un appui long ouvre l'historique (24 au plus). La sonnette : trace sa zone sur la porte d'entrée ; elle s'allume avec la photo quand on sonne.",
    el("ol", { class: "drawing-items" }, ...zoneRows),
    el("div", { class: "drawing-zone-buttons" }, addZone, addBell),
  );
  zones.classList.add("drawing-zones");

  // Les courbes de la colonne de gauche.
  const chartRows = draft.charts.map((chart, index) => {
    const entity = el("input", { type: "text", list: "entity-list", value: chart.entity_id, placeholder: "sensor.puissance_nas", autocapitalize: "none", spellcheck: "false" });
    entity.addEventListener("change", () => { chart.entity_id = entity.value.trim(); });
    const name = el("input", { type: "text", value: chart.name ?? "", placeholder: "(le nom de l'appareil)", maxlength: "60" });
    name.addEventListener("change", () => { if (name.value.trim()) chart.name = name.value.trim(); else delete chart.name; });
    const color = el("select", {}, ...CHART_COLORS.map(([value, text]) => el("option", { value, text })));
    color.value = chart.color;
    color.addEventListener("change", () => { chart.color = color.value; });
    const hours = el("select", {}, ...CHART_HOURS.map(([value, text]) => el("option", { value: String(value), text })));
    hours.value = String(chart.hours);
    hours.addEventListener("change", () => { chart.hours = Number(hours.value); });
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", () => { draft.charts.splice(index, 1); refresh(); });
    return el("li", { class: "drawing-item" },
      el("div", { class: "drawing-item-head" }, el("b", { text: `Courbe ${index + 1}` }), remove),
      el("div", { class: "drawing-grid" }, field("Capteur", entity), field("Nom affiché", name), field("Couleur", color), field("Durée", hours)));
  });
  const addChart = el("button", { type: "button", class: "ghost small", text: "+ Ajouter une courbe" });
  addChart.disabled = draft.charts.length >= 6;
  addChart.addEventListener("click", () => {
    draft.charts.push({ entity_id: "", color: CHART_COLORS[draft.charts.length % CHART_COLORS.length][0], hours: 24 });
    refresh();
    $("#drawing-form fieldset:last-of-type .drawing-item:last-child input")?.focus();
  });
  const charts = drawingFieldset(
    "Courbes",
    "Dans la colonne de gauche de l'onglet : la puissance d'un appareil, une température… sur 6 heures, 24 heures ou 7 jours (6 au plus). Toucher une courbe ouvre son historique.",
    el("ol", { class: "drawing-items" }, ...chartRows),
    addChart,
  );

  $("#drawing-form").replaceChildren(pagesPart(), ai, images, cutPart(draft, refresh), backgrounds, effects, layers, labels, zones, actions, charts);
}

/** Un identifiant d'onglet à partir de son nom (« Salle de jeux » → salle-de-jeux). */
function pageId(name) {
  const base = name.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 30) || "onglet";
  let id = base;
  for (let n = 2; drawingEdit.pages.some((page) => page.id === id); n += 1) id = `${base.slice(0, 27)}-${n}`;
  return id;
}

/** Les onglets : lequel on règle, son nom, ses pièces ; en ajouter, en retirer. */
function pagesPart() {
  const refresh = () => renderDrawingEditor();
  const draft = drawingEdit.draft;
  const choose = el("select", { "aria-label": "Onglet à régler" }, ...drawingEdit.pages.map((page) => el("option", { value: page.id, text: page.name })));
  choose.value = draft.id;
  choose.addEventListener("change", () => {
    drawingEdit.draft = drawingEdit.pages.find((page) => page.id === choose.value) ?? drawingEdit.pages[0];
    drawingEdit.placing = -1;
    drawingEdit.tracing = -1;
    refresh();
  });
  const add = el("button", { type: "button", class: "ghost small", text: "+ Nouvel onglet" });
  add.disabled = drawingEdit.pages.length >= 8;
  add.addEventListener("click", () => {
    const page = { id: pageId("Nouvel onglet"), name: "Nouvel onglet", areas: [], ...emptyDrawing() };
    drawingEdit.pages.push(page);
    drawingEdit.draft = page;
    refresh();
    $("#drawing-form .drawing-part input")?.select();
  });
  const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer cet onglet" });
  remove.disabled = drawingEdit.pages.length === 1;
  remove.addEventListener("click", () => {
    drawingEdit.pages = drawingEdit.pages.filter((page) => page !== draft);
    drawingEdit.draft = drawingEdit.pages[0];
    refresh();
  });
  const first = el("button", { type: "button", class: "ghost small", text: "Mettre en premier" });
  first.disabled = drawingEdit.pages[0] === draft;
  first.addEventListener("click", () => {
    drawingEdit.pages = [draft, ...drawingEdit.pages.filter((page) => page !== draft)];
    refresh();
  });
  const name = el("input", { type: "text", value: draft.name, maxlength: "30", "aria-label": "Nom de l'onglet" });
  // Sans tout redessiner : « change » arrive quand le champ perd le focus.
  name.addEventListener("change", () => {
    draft.name = name.value.trim() || draft.name;
    name.value = draft.name;
    if (choose.selectedOptions[0]) choose.selectedOptions[0].textContent = draft.name;
  });
  // Les pièces de cet onglet (aucune cochée : toutes).
  const rooms = el("div", { class: "drawing-rooms" }, ...areas.map((area) => {
    const box = el("input", { type: "checkbox" });
    box.checked = draft.areas.includes(area.name);
    box.addEventListener("change", () => {
      draft.areas = box.checked ? [...draft.areas, area.name] : draft.areas.filter((a) => a !== area.name);
    });
    return el("label", { class: "check" }, box, ` ${area.name}`);
  }));
  return drawingFieldset(
    "Onglets",
    "Chaque onglet a son dessin, ses boutons, et les pièces qu'il montre (aucune cochée : toute la maison). Le premier s'ouvre par défaut.",
    el("div", { class: "drawing-row" }, choose, add),
    el("div", { class: "drawing-grid" }, field("Nom de l'onglet", name)),
    areas.length > 0 ? field("Pièces montrées", rooms) : "",
    el("div", { class: "drawing-row" }, first, remove),
  );
}

// Poser une valeur : un toucher sur l'aperçu donne sa place, en % de l'image.
$("#drawing-preview").addEventListener("click", (event) => {
  const index = drawingEdit.placing;
  if (index < 0 || !drawingEdit.draft) return;
  const box = $("#drawing-preview .house-base")?.getBoundingClientRect();
  if (!box || box.width === 0) return;
  const percent = (value) => Math.round(Math.min(100, Math.max(0, value)) * 10) / 10;
  const label = drawingEdit.draft.labels[index];
  label.x = percent(((event.clientX - box.left) / box.width) * 100);
  label.y = percent(((event.clientY - box.top) / box.height) * 100);
  drawingEdit.placing = -1;
  renderDrawingEditor();
});

// Tracer une zone : on fait glisser sur l'aperçu, du premier coin à l'autre.
const zoneDraw = { start: null, rect: null };

function previewPercent(event) {
  const box = $("#drawing-preview .house-base")?.getBoundingClientRect();
  if (!box || box.width === 0) return null;
  const clamp = (value) => Math.min(100, Math.max(0, value));
  return { x: clamp(((event.clientX - box.left) / box.width) * 100), y: clamp(((event.clientY - box.top) / box.height) * 100) };
}

function traceBox(a, b) {
  const round = (value) => Math.round(value * 10) / 10;
  const x = round(Math.min(a.x, b.x));
  const y = round(Math.min(a.y, b.y));
  return { x, y, w: round(Math.min(100 - x, Math.abs(a.x - b.x))), h: round(Math.min(100 - y, Math.abs(a.y - b.y))) };
}

$("#drawing-preview").addEventListener("pointerdown", (event) => {
  if (drawingEdit.tracing < 0 || !drawingEdit.draft) return;
  const start = previewPercent(event);
  if (!start) return;
  event.preventDefault();
  zoneDraw.start = start;
  zoneDraw.rect = el("span", { class: "house-zone edit tracing" });
  placeZone(zoneDraw.rect, { ...start, w: 0, h: 0 });
  $("#drawing-preview").append(zoneDraw.rect);
  $("#drawing-preview").setPointerCapture(event.pointerId);
});
$("#drawing-preview").addEventListener("pointermove", (event) => {
  if (!zoneDraw.start) return;
  const point = previewPercent(event);
  if (point) placeZone(zoneDraw.rect, traceBox(zoneDraw.start, point));
});
for (const type of ["pointerup", "pointercancel"]) {
  $("#drawing-preview").addEventListener(type, (event) => {
    if (!zoneDraw.start) return;
    const point = type === "pointerup" ? previewPercent(event) : null;
    const box = point ? traceBox(zoneDraw.start, point) : null;
    zoneDraw.start = null;
    zoneDraw.rect?.remove();
    zoneDraw.rect = null;
    if (!box || box.w < 1 || box.h < 1) return; // un simple toucher : on recommence
    Object.assign(drawingEdit.draft.zones[drawingEdit.tracing], box);
    drawingEdit.tracing = -1;
    renderDrawingEditor();
  });
}

$("#drawing-save").addEventListener("click", async () => {
  if (!drawingEdit.pages) return;
  for (const page of drawingEdit.pages) {
    page.actions = page.actions.filter((id) => id !== "");
    page.charts = page.charts.filter((chart) => chart.entity_id !== "");
    page.zones = page.zones.filter((zone) => (zone.area ?? zone.entity_id ?? "") !== "");
  }
  const button = $("#drawing-save");
  button.disabled = true;
  const { status, data } = await api("PUT", "/api/home/drawing", { pages: drawingEdit.pages }).catch(() => ({ status: 0, data: {} }));
  button.disabled = false;
  if (status !== 200) return drawingStatus(data.error ?? "Enregistrement impossible.", "error");
  house.pages = cleanPages(data.config);
  selectPage(drawingEdit.draft.id);
  $("#drawing").close();
  flash("Dessin de la maison enregistré");
});

for (const id of ["#drawing-close", "#drawing-cancel"]) $(id).addEventListener("click", () => $("#drawing").close());
$("#open-drawing").addEventListener("click", openDrawingEditor);
