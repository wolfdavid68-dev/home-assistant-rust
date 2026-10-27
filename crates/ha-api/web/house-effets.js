// La mise en page « tableau » — les effets animés : pluie, neige, grêle,
// brouillard et vent sur le dessin, et le même ciel derrière les chiffres
// de la température.
//
// Un des fichiers de house.js, chargé juste après lui par index.html. Les
// scripts partagent leurs déclarations : `house`, `el`, `entities`… viennent
// de house.js et de app.js. Pas d'innerHTML ici non plus.
"use strict";

// ---------------------------------------------------------------- Effets animés
// Sur le dessin, le temps qu'il fait : la pluie, la neige, un éclair… Un seul
// <canvas> par dessin, et une seule boucle (requestAnimationFrame) pour tous,
// arrêtée quand la page est cachée. Sans animation si l'appareil demande de
// réduire les mouvements : une image fixe, sans éclair.

const EFFECTS = [
  ["pluie", "Pluie"], ["averses", "Averses"], ["orage", "Orage"], ["grele", "Grêle"], ["neige", "Neige"],
  ["pluie-neige", "Pluie et neige"], ["brouillard", "Brouillard"], ["vent", "Vent"], ["etoiles", "Étoiles (nuit claire)"],
];
const EFFECT_LEVELS = [["discret", "Discrète"], ["", "Normale"], ["fort", "Forte"]];
const WEATHER_EFFECT = { "pluie": "pluie", "averses": "averses", "orage": "orage", "grele": "grele", "neige": "neige", "pluie-neige": "pluie-neige", "brouillard": "brouillard", "vent": "vent" };
// Particules pour 10 000 px² du dessin, à l'intensité normale.
const FX_DENSITY = { "pluie": 2.2, "averses": 4, "orage": 3.2, "grele": 1.6, "neige": 1.7, "pluie-neige": 2.2, "brouillard": 0, "vent": 0.3, "etoiles": 0.4 };
const FX_LEVEL = { "discret": 0.5, "": 1, "fort": 1.8 };
const fxViews = new Set();
const fxMotion = matchMedia("(prefers-reduced-motion: reduce)");
let fxFrame = 0;
let fxLast = 0;

/** L'effet d'un onglet : celui imposé, sinon celui du temps affiché (le même
 *  que l'ambiance : la nuit claire donne des étoiles). */
function effectOf(page) {
  if (page.effects === "off") return null;
  if (page.effects) return page.effects;
  const ambiance = ambianceOf(page);
  const cut = ambiance.indexOf("-");
  if (cut < 0) return ambiance === "nuit" || (ambiance === "crepuscule" && houseNight()) ? "etoiles" : null;
  return lookup(WEATHER_EFFECT, ambiance.slice(cut + 1)) ?? null;
}

function makeFx() {
  const canvas = el("canvas", { class: "house-fx", "aria-hidden": "true" });
  return { canvas, ctx: canvas.getContext("2d"), kind: null, level: "", w: 0, h: 0, dpr: 1, ink: "", parts: [], fog: [], flash: 0, nextFlash: 0 };
}

function setFx(fx, kind, level) {
  if (fx.kind === kind && fx.level === level) return;
  fx.kind = kind;
  fx.level = level;
  fx.w = 0; // à remesurer, et à repeupler
  if (kind === null) {
    fxViews.delete(fx);
    fx.ctx?.clearRect(0, 0, fx.canvas.width, fx.canvas.height);
  } else {
    fxViews.add(fx);
  }
  fxSchedule();
}

function fxSchedule() {
  if (fxFrame === 0 && fxViews.size > 0 && !document.hidden) fxFrame = requestAnimationFrame(fxTick);
}

function fxTick(now) {
  fxFrame = 0;
  const still = fxMotion.matches;
  const dt = still || fxLast === 0 ? 0 : Math.min(0.05, (now - fxLast) / 1000);
  fxLast = now;
  for (const fx of fxViews) {
    if (!fx.canvas.isConnected) { fxViews.delete(fx); continue; }
    (fx.step ?? fxStep)(fx, dt, now / 1000, still);
  }
  if (!still) fxSchedule();
}

document.addEventListener("visibilitychange", () => { fxLast = 0; fxSchedule(); });
fxMotion.addEventListener("change", () => { fxLast = 0; fxSchedule(); });

const rand = (a, b) => a + Math.random() * (b - a);

/** Une particule, n'importe où (ou en haut, `top`, quand elle revient). */
function fxPart(fx, type, top = false) {
  const { w, h } = fx;
  const p = { type, x: rand(-0.1 * w, 1.05 * w), y: top ? rand(-0.15 * h, -5) : rand(-0.1 * h, h), phase: rand(0, Math.PI * 2) };
  if (type === "rain") Object.assign(p, { vy: rand(650, 950), vx: rand(110, 160), len: rand(10, 18), a: rand(0.35, 0.6) });
  else if (type === "pour") Object.assign(p, { vy: rand(900, 1300), vx: rand(160, 220), len: rand(16, 28), a: rand(0.3, 0.6) });
  else if (type === "hail") Object.assign(p, { vy: rand(520, 760), vx: rand(40, 80), r: rand(1.2, 2.1), a: rand(0.6, 0.9) });
  else if (type === "snow") Object.assign(p, { vy: rand(25, 65), vx: rand(-8, 12), r: rand(1, 3), a: rand(0.55, 0.95), sway: rand(10, 26), freq: rand(0.4, 1.1) });
  else if (type === "streak") Object.assign(p, { x: top ? rand(-0.3 * w, -20) : rand(-0.2 * w, w), y: rand(0, h), vx: rand(480, 760), vy: rand(-10, 20), len: rand(30, 70), a: rand(0.12, 0.28) });
  else if (type === "leaf") Object.assign(p, { x: top ? rand(-0.3 * w, -10) : rand(0, w), y: rand(0, h), vx: rand(140, 260), vy: rand(-15, 35), r: rand(2.5, 4.5), a: rand(0.55, 0.85), spin: rand(-6, 6), sway: rand(8, 20), freq: rand(0.8, 1.8) });
  else if (type === "star") Object.assign(p, { y: rand(0.02 * h, 0.38 * h), r: rand(0.5, 1.5), a: rand(0.35, 0.9), freq: rand(0.6, 1.8) });
  return p;
}

/** Peuple l'effet pour la taille du dessin. */
function fxSeed(fx) {
  const area = (fx.w * fx.h) / 10_000;
  const phone = fx.w < 600 ? 0.7 : 1;
  const count = Math.min(900, Math.round(area * (FX_DENSITY[fx.kind] ?? 0) * (FX_LEVEL[fx.level] ?? 1) * phone * (fx.boost ?? 1)));
  const types = {
    "pluie": ["rain"], "averses": ["pour"], "orage": ["pour", "rain"], "grele": ["hail", "rain"], "neige": ["snow"],
    "pluie-neige": ["rain", "snow"], "vent": ["streak", "streak", "leaf"], "etoiles": ["star"], "brouillard": [],
  }[fx.kind] ?? [];
  fx.parts = types.length === 0 ? [] : Array.from({ length: count }, (_, i) => fxPart(fx, types[i % types.length]));
  fx.fog = fx.kind === "brouillard" || fx.kind === "pluie-neige"
    ? Array.from({ length: Math.round(4 * (FX_LEVEL[fx.level] ?? 1)) + 1 }, () => ({ x: rand(0, fx.w), y: rand(0.35 * fx.h, fx.h), rx: rand(0.25, 0.5) * fx.w, ry: rand(0.08, 0.16) * fx.h, vx: rand(6, 18) * (Math.random() < 0.5 ? -1 : 1), a: rand(0.1, 0.2) * (fx.kind === "brouillard" ? 1 : 0.5) }))
    : [];
  fx.nextFlash = performance.now() / 1000 + rand(2, 6);
}

function fxStep(fx, dt, t, still) {
  const { canvas, ctx } = fx;
  const cw = canvas.clientWidth;
  const ch = canvas.clientHeight;
  if (cw === 0 || ch === 0 || !ctx) return;
  if (cw !== fx.w || ch !== fx.h) {
    fx.dpr = Math.min(2, devicePixelRatio || 1);
    canvas.width = Math.round(cw * fx.dpr);
    canvas.height = Math.round(ch * fx.dpr);
    fx.w = cw;
    fx.h = ch;
    fx.ink = getComputedStyle(canvas).getPropertyValue("--fx-ink").trim() || "225 235 255";
    fxSeed(fx);
  }
  ctx.setTransform(fx.dpr, 0, 0, fx.dpr, 0, 0);
  ctx.clearRect(0, 0, fx.w, fx.h);
  fxPaint(fx, dt, t, still);
}

/** Dessine l'effet par-dessus ce qu'il y a déjà (fx.ctx, à la taille fx.w × fx.h). */
function fxPaint(fx, dt, t, still) {
  const { ctx, w, h, ink } = fx;

  // Le brouillard : de grands voiles flous qui dérivent.
  for (const f of fx.fog) {
    f.x += f.vx * dt;
    if (f.x - f.rx > w) f.x = -f.rx; else if (f.x + f.rx < 0) f.x = w + f.rx;
    ctx.save();
    ctx.translate(f.x, f.y);
    ctx.scale(1, f.ry / f.rx);
    const g = ctx.createRadialGradient(0, 0, 0, 0, 0, f.rx);
    g.addColorStop(0, `rgb(${ink} / ${f.a})`);
    g.addColorStop(1, `rgb(${ink} / 0)`);
    ctx.fillStyle = g;
    ctx.fillRect(-f.rx, -f.rx, f.rx * 2, f.rx * 2);
    ctx.restore();
  }

  // Les traits (pluie, vent) : un seul tracé par opacité, c'est bien plus rapide.
  ctx.lineCap = "round";
  for (const p of fx.parts) {
    if (p.type === "star" || p.type === "leaf" || p.type === "snow" || p.type === "hail") continue;
    p.x += p.vx * dt;
    p.y += p.vy * dt;
    if (p.y - p.len > h || p.x - p.len > w) Object.assign(p, fxPart(fx, p.type, true));
  }
  for (const [type, width] of [["rain", 1], ["pour", 1.2], ["streak", 1]]) {
    ctx.beginPath();
    let alpha = 0;
    let n = 0;
    for (const p of fx.parts) {
      if (p.type !== type) continue;
      const speed = Math.hypot(p.vx, p.vy);
      ctx.moveTo(p.x, p.y);
      ctx.lineTo(p.x - (p.vx / speed) * p.len, p.y - (p.vy / speed) * p.len);
      alpha += p.a;
      n += 1;
    }
    if (n === 0) continue;
    ctx.strokeStyle = `rgb(${ink} / ${(alpha / n).toFixed(2)})`;
    ctx.lineWidth = width;
    ctx.stroke();
  }

  // Les grains (neige, grêle).
  ctx.fillStyle = `rgb(${ink} / 0.85)`;
  ctx.beginPath();
  for (const p of fx.parts) {
    if (p.type !== "snow" && p.type !== "hail") continue;
    p.y += p.vy * dt;
    p.x += p.vx * dt;
    const x = p.type === "snow" ? p.x + Math.sin(t * p.freq + p.phase) * p.sway : p.x;
    if (p.y - p.r > h) Object.assign(p, fxPart(fx, p.type, true));
    ctx.moveTo(x + p.r, p.y);
    ctx.arc(x, p.y, p.r, 0, Math.PI * 2);
  }
  ctx.fill();

  // Les feuilles (vent) et les étoiles (nuit claire), une à une.
  for (const p of fx.parts) {
    if (p.type === "leaf") {
      p.x += p.vx * dt;
      p.y += (p.vy + Math.sin(t * p.freq + p.phase) * p.sway) * dt;
      if (p.x - p.r > w || p.y > h + 10) Object.assign(p, fxPart(fx, "leaf", true));
      ctx.save();
      ctx.translate(p.x, p.y);
      ctx.rotate(t * p.spin + p.phase);
      ctx.fillStyle = `rgb(${ink} / ${p.a})`;
      ctx.beginPath();
      ctx.ellipse(0, 0, p.r, p.r * 0.45, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
    } else if (p.type === "star") {
      const twinkle = still ? 1 : 0.55 + 0.45 * Math.sin(t * p.freq + p.phase);
      ctx.fillStyle = `rgb(${ink} / ${(p.a * twinkle).toFixed(2)})`;
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  // L'orage : un éclair de temps en temps, en deux temps (jamais si l'appareil
  // demande de réduire les mouvements).
  if (fx.kind === "orage" && !still) {
    if (t >= fx.nextFlash) {
      fx.flash = 1;
      fx.second = t + rand(0.12, 0.22);
      fx.nextFlash = t + rand(5, 13) / (FX_LEVEL[fx.level] ?? 1);
    }
    if (fx.second && t >= fx.second) { fx.flash = Math.max(fx.flash, 0.7); fx.second = 0; }
    if (fx.flash > 0.01) {
      ctx.fillStyle = `rgb(255 255 255 / ${(fx.flash * 0.32).toFixed(3)})`;
      ctx.fillRect(0, 0, w, h);
      fx.flash *= Math.exp(-dt * 9);
    }
  }
}

// ---------------------------------------------------------------- Le temps dans les chiffres
// Dehors, les chiffres de la température sont des fenêtres sur le temps qu'il
// fait : derrière eux, le ciel du moment (jour, crépuscule, nuit) et le même
// moteur que les effets du dessin (pluie, neige, grêle, brouillard, vent,
// orage, dont l'éclair allume la pastille) ; le soleil et ses rayons par beau
// temps, les étoiles d'une nuit claire. Un seul dessin, gardé d'un rendu à
// l'autre : la pluie ne repart pas de zéro à chaque mise à jour.

/** Le ciel, de haut en bas, le jour. */
const SKY_DAY = {
  "soleil": ["#8fd3ff", "#3d8fe6"], "eclaircies": ["#a5d4f4", "#5b92c8"], "nuageux": ["#b8c4d1", "#6c7c8f"],
  "brouillard": ["#cfd6dc", "#8b97a3"], "vent": ["#9cc3e0", "#557fa6"], "pluie": ["#a3b3c7", "#5a6b84"],
  "averses": ["#97a8be", "#4f5f78"], "orage": ["#8e9ab8", "#434c6e"], "grele": ["#a9b6c8", "#5b6a80"],
  "neige": ["#aebfd6", "#5e7899"], "pluie-neige": ["#a3b1c2", "#56657a"],
};
const SKY_HOT = ["#ffd765", "#ff5f2e"]; // le grand soleil, quand il fait chaud
const SKY_HOT_FROM = 20;
const SKY_DUSK = ["#3f3b7d", "#ff8a5c"];
const SKY_NIGHT = ["#2a3d80", "#5a7ed0"];
const SKY_NIGHT_GREY = ["#4a5670", "#262e40"];
const SKY_EFFECT = { "brouillard": "brouillard", "vent": "vent", "pluie": "pluie", "averses": "averses", "orage": "orage", "grele": "grele", "neige": "neige", "pluie-neige": "pluie-neige" };
// Le dessin est petit : bien plus de gouttes au cm², qui tombent moins vite.
const SKY_BOOST = 50;
const SKY_PACE = 0.35;
// Autour des chiffres, la place du halo (en px, comme dans style.css).
const SKY_PAD_X = 12;
const SKY_PAD_Y = 8;

let sky = null;

function skyInit() {
  const layer = () => { const canvas = document.createElement("canvas"); return { canvas, ctx: canvas.getContext("2d") }; };
  const shown = el("canvas", { class: "g-sky", "aria-hidden": "true" });
  const scene = layer();
  sky = {
    shown, view: shown.getContext("2d"), scene, cut: layer(), edge: layer(), haze: layer(),
    key: "", text: "", font: null, chip: null, colors: SKY_DAY.soleil, sun: false, clouds: 0, bolt: null, acc: 0, glow: 0,
    // Pour le moteur des effets : il dessine dans la scène, et vit tant que la
    // pastille est à l'écran.
    fx: { canvas: shown, ctx: scene.ctx, kind: null, level: "", w: 0, h: 0, dpr: 1, ink: "255 255 255", parts: [], fog: [], flash: 0, nextFlash: 0, boost: SKY_BOOST, step: skyStep },
  };
  document.fonts?.ready.then(() => { sky.font = null; fxSchedule(); });
}

/** Le ciel du moment, pour une température dehors. */
function skyOf(value) {
  const weather = weatherKind();
  const night = houseNight();
  const fair = weather === "soleil" || weather === "eclaircies";
  const calm = fair || weather === "nuageux";
  let colors = SKY_DAY[weather] ?? SKY_DAY.soleil;
  if (calm && houseDusk()) colors = SKY_DUSK;
  else if (night) colors = fair ? SKY_NIGHT : SKY_NIGHT_GREY;
  else if (weather === "soleil" && value >= SKY_HOT_FROM) colors = SKY_HOT;
  return {
    colors,
    kind: night && fair ? "etoiles" : lookup(SKY_EFFECT, weather) ?? null,
    sun: !night && fair && colors !== SKY_DUSK,
    clouds: weather === "eclaircies" ? 2 : weather === "nuageux" ? 4 : 0,
  };
}

/** Met la scène dans la pastille qu'on vient de faire. */
function skyMount(chip, strong, value) {
  if (sky === null) skyInit();
  const scene = skyOf(value);
  const key = `${scene.kind}|${scene.colors}|${scene.sun}|${scene.clouds}`;
  if (key !== sky.key) {
    Object.assign(sky, { key, colors: scene.colors, sun: scene.sun, clouds: scene.clouds, bolt: null });
    sky.fx.kind = scene.kind;
    sky.fx.flash = 0;
    sky.fx.w = 0; // à repeupler
  }
  sky.text = number.format(value);
  sky.font = null;
  sky.chip = chip;
  sky.glow = -1;
  chip.classList.add("sky");
  chip.style.setProperty("--sky", scene.colors[1]);
  strong.append(sky.shown);
  fxViews.add(sky.fx);
  fxSchedule();
}

/** Le moteur peuplé à la taille de la pastille, avec ses nuages. */
function skySeed(fx) {
  fxSeed(fx);
  const { w, h } = fx;
  for (const p of fx.parts) if (p.type === "star") p.y = rand(0, h);
  if (sky.clouds > 0) {
    fx.fog = Array.from({ length: sky.clouds }, (_, i) => ({
      x: rand(0, w), y: rand(0.1, 0.55) * h, rx: rand(0.28, 0.45) * w, ry: rand(0.14, 0.22) * h,
      vx: rand(3, 7) * (i % 2 ? -1 : 1), a: houseNight() ? rand(0.12, 0.2) : rand(0.4, 0.6),
    }));
  }
}

function skyStep(fx, dt, t, still) {
  const cw = fx.canvas.clientWidth;
  const ch = fx.canvas.clientHeight;
  if (cw === 0 || ch === 0 || !sky.view) return;
  sky.acc += dt;
  // Trente images par seconde suffisent, pour si peu.
  if (!still && cw === fx.w && ch === fx.h && sky.font && sky.acc < 1 / 30) return;
  const step = sky.acc * SKY_PACE;
  sky.acc = 0;
  if (cw !== fx.w || ch !== fx.h) {
    fx.dpr = Math.min(2, devicePixelRatio || 1);
    const W = Math.round(cw * fx.dpr);
    const H = Math.round(ch * fx.dpr);
    for (const c of [fx.canvas, sky.scene.canvas, sky.cut.canvas, sky.edge.canvas]) { c.width = W; c.height = H; }
    sky.haze.canvas.width = Math.ceil(W / 8);
    sky.haze.canvas.height = Math.ceil(H / 8);
    fx.w = cw;
    fx.h = ch;
    skySeed(fx);
  }
  if (!sky.font) skyFont();
  const { w, h, dpr } = fx;
  const W = fx.canvas.width;
  const H = fx.canvas.height;

  // La scène : le ciel, le soleil, puis le moteur (et l'éclair).
  const s = sky.scene.ctx;
  s.setTransform(dpr, 0, 0, dpr, 0, 0);
  const air = s.createLinearGradient(0, 0, 0, h);
  air.addColorStop(0, sky.colors[0]);
  air.addColorStop(1, sky.colors[1]);
  s.fillStyle = air;
  s.fillRect(0, 0, w, h);
  if (sky.sun) skySun(s, w, h, still ? 0 : t);
  fxPaint(fx, step, t, still);
  if (fx.kind === "orage") skyBolt(s, fx, w, h);

  // Les chiffres, remplis de la scène.
  const c = sky.cut.ctx;
  c.globalCompositeOperation = "source-over";
  c.setTransform(1, 0, 0, 1, 0, 0);
  c.clearRect(0, 0, W, H);
  c.setTransform(dpr, 0, 0, dpr, 0, 0);
  c.fillStyle = "#fff";
  skyText(c, false);
  c.setTransform(1, 0, 0, 1, 0, 0);
  c.globalCompositeOperation = "source-in";
  c.drawImage(sky.scene.canvas, 0, 0);

  // Le contour, seulement au-dehors des chiffres (sinon les traits se
  // croisent là où les chiffres se touchent).
  const e = sky.edge.ctx;
  e.globalCompositeOperation = "source-over";
  e.setTransform(1, 0, 0, 1, 0, 0);
  e.clearRect(0, 0, W, H);
  e.setTransform(dpr, 0, 0, dpr, 0, 0);
  e.strokeStyle = sky.font.edge;
  e.lineWidth = 1.2;
  e.lineJoin = "round";
  skyText(e, true);
  e.globalCompositeOperation = "destination-out";
  e.fillStyle = "#000";
  skyText(e, false);

  // Le halo (la scène très floue : en tout petit, puis agrandie), le
  // contour, les chiffres.
  const z = sky.haze.ctx;
  z.clearRect(0, 0, z.canvas.width, z.canvas.height);
  z.drawImage(sky.cut.canvas, 0, 0, z.canvas.width, z.canvas.height);
  const v = sky.view;
  v.setTransform(1, 0, 0, 1, 0, 0);
  v.clearRect(0, 0, W, H);
  v.imageSmoothingEnabled = true;
  v.imageSmoothingQuality = "high";
  v.globalAlpha = 0.45;
  v.drawImage(sky.haze.canvas, 0, 0, W, H);
  v.globalAlpha = 0.5;
  v.drawImage(sky.edge.canvas, 0, 0);
  v.globalAlpha = 1;
  v.drawImage(sky.cut.canvas, 0, 0);

  // L'éclair allume toute la pastille.
  const glow = fx.kind === "orage" ? Math.round(fx.flash * 50) / 50 : 0;
  if (glow !== sky.glow) {
    sky.glow = glow;
    sky.chip?.style.setProperty("--flash", String(glow));
  }
}

/** La police des chiffres, celle de la page (lue une fois à l'écran). */
function skyFont() {
  const css = getComputedStyle(sky.shown.parentElement);
  const size = parseFloat(css.fontSize) || 24;
  const spacing = parseFloat(css.letterSpacing) || 0;
  const font = `${css.fontWeight} ${size}px ${css.fontFamily}`;
  const ctx = sky.view;
  ctx.font = font;
  const m = ctx.measureText("0");
  const ascent = m.fontBoundingBoxAscent ?? size * 0.8;
  const descent = m.fontBoundingBoxDescent ?? size * 0.2;
  sky.font = {
    font, spacing,
    small: `${css.fontWeight} ${size / 2}px ${css.fontFamily}`,
    // Ligne de hauteur 1 : la ligne de base des chiffres, puis celle du « ° »
    // (en haut, à demi-taille, comme le <sup>).
    base: SKY_PAD_Y + (size - ascent - descent) / 2 + ascent,
    sup: SKY_PAD_Y + 0.39 * size + (ascent - descent) / 4,
    edge: css.getPropertyValue("--ink").trim() || "#fff",
  };
}

function skyText(ctx, stroke) {
  const f = sky.font;
  const spaced = "letterSpacing" in ctx;
  const draw = (text, x, y) => (stroke ? ctx.strokeText(text, x, y) : ctx.fillText(text, x, y));
  ctx.textBaseline = "alphabetic";
  ctx.textAlign = "left";
  ctx.font = f.font;
  if (spaced) ctx.letterSpacing = `${f.spacing}px`;
  draw(sky.text, SKY_PAD_X, f.base);
  const x = SKY_PAD_X + ctx.measureText(sky.text).width + (spaced ? 0 : f.spacing * sky.text.length) + 1;
  ctx.font = f.small;
  if (spaced) ctx.letterSpacing = "0px";
  draw("°", x, f.sup);
}

/** Le soleil : un disque blanc, et ses rayons qui tournent doucement. */
function skySun(ctx, w, h, t) {
  const x = w * 0.66;
  const y = h * 0.4;
  const r = h * 0.22;
  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(t * 0.25);
  ctx.fillStyle = "rgb(255 255 255 / 0.28)";
  ctx.beginPath();
  for (let i = 0; i < 12; i += 1) {
    const a = (i / 12) * Math.PI * 2;
    ctx.moveTo(Math.cos(a - 0.09) * r * 1.3, Math.sin(a - 0.09) * r * 1.3);
    ctx.lineTo(Math.cos(a) * r * 4.2, Math.sin(a) * r * 4.2);
    ctx.lineTo(Math.cos(a + 0.09) * r * 1.3, Math.sin(a + 0.09) * r * 1.3);
  }
  ctx.fill();
  ctx.restore();
  const g = ctx.createRadialGradient(x, y, 0, x, y, r * 1.8);
  g.addColorStop(0, "rgb(255 255 255 / 1)");
  g.addColorStop(0.5, "rgb(255 250 225 / 0.95)");
  g.addColorStop(1, "rgb(255 240 190 / 0)");
  ctx.fillStyle = g;
  ctx.fillRect(x - r * 2, y - r * 2, r * 4, r * 4);
}

/** L'éclair : un trait brisé, le temps du flash. */
function skyBolt(ctx, fx, w, h) {
  if (fx.flash < 0.15) { sky.bolt = null; return; }
  if (sky.bolt === null && fx.flash > 0.6) {
    let x = rand(0.25, 0.75) * w;
    sky.bolt = [[x, -2]];
    for (let i = 1; i <= 5; i += 1) { x += rand(-0.09, 0.09) * w; sky.bolt.push([x, (i / 5) * h]); }
  }
  if (sky.bolt === null) return;
  ctx.strokeStyle = `rgb(255 255 255 / ${Math.min(1, fx.flash).toFixed(2)})`;
  ctx.lineWidth = 1.6;
  ctx.lineJoin = "round";
  ctx.beginPath();
  sky.bolt.forEach(([x, y], i) => (i ? ctx.lineTo(x, y) : ctx.moveTo(x, y)));
  ctx.stroke();
}

/** Dehors : seulement la température, en grand, ses chiffres ouverts sur le
 *  temps qu'il fait (voir plus haut). Sur le plan, à l'encre, avec un reflet
 *  qui monte quand ça se réchauffe et descend quand ça se rafraîchit. Toucher :
 *  l'historique. */
function outsideGauge(value) {
  const probe = entities.get("sensor.temperature_exterieure");
  const id = probe && Number(probe.state) === value ? probe.entity_id : null;
  const history = id !== null && account.permissions.history !== false;
  const trend = history ? outsideTrend(id, value) : null;
  const said = [`Dehors : ${number.format(value)} °C`];
  if (trend) said.push(trend.text);
  const classes = ["gauge", "g-outside", trend?.way, value <= 0 ? "frost" : null].filter(Boolean).join(" ");
  const temp = el("strong", { class: "g-temp" }, number.format(value), el("sup", { text: "°" }));
  const chip = el("span", { class: classes, title: said.join(", "), "aria-label": said.join(", ") },
    el("b", { class: "g-now" }, el("small", { text: value <= 0 ? "Dehors · gel" : "Dehors" }), temp));
  chip.style.setProperty("--temp", tempColor(value));
  skyMount(chip, temp, value);
  if (history) {
    chip.setAttribute("role", "button");
    chip.tabIndex = 0;
    chip.addEventListener("click", () => openHistory(id));
    chip.addEventListener("keydown", (event) => { if (event.key === "Enter") openHistory(id); });
  }
  return chip;
}

/** La tendance : l'écart avec la valeur d'il y a une heure environ ; rien si
 *  c'est stable (moins de 0,3 °C par heure) ou si on ne sait pas. */
function outsideTrend(id, value) {
  const now = Date.now();
  const hourAgo = (seriesFor({ entity_id: id, hours: 24 }) ?? [])
    .filter(([t]) => t >= now - 2 * 3_600_000 && t <= now - 30 * 60_000)
    .sort((a, b) => Math.abs(a[0] - (now - 3_600_000)) - Math.abs(b[0] - (now - 3_600_000)))[0];
  if (!hourAgo) return null;
  const delta = (value - hourAgo[1]) / ((now - hourAgo[0]) / 3_600_000);
  if (Math.abs(delta) <= 0.3) return null;
  const amount = number.format(Math.abs(Math.round(delta * 10) / 10));
  return delta > 0
    ? { way: "up", text: `en hausse de ${amount} °C par heure` }
    : { way: "down", text: `en baisse de ${amount} °C par heure` };
}

function renderGauges() {
  const box = $("#gauges");
  const items = [];
  // D'abord la sonde de la pompe à chaleur, posée dehors ; sinon la météo.
  const outsideTemp = outsideTemperature();
  if (Number.isFinite(outsideTemp)) items.push(outsideGauge(outsideTemp));
  const temps = areas.map(roomInfo).map((info) => info.temperature).filter((t) => Number.isFinite(t));
  if (temps.length > 0) {
    items.push(gauge(ICONS.home, "Dedans", `${number.format(temps.reduce((a, b) => a + b, 0) / temps.length)} °C`, "inside"));
  }
  const power = entities.get("sensor.linky_puissance");
  if (power && Number.isFinite(Number(power.state))) {
    items.push(gauge(ICONS.flame, "Puissance", `${number.format(Number(power.state))} ${power.attributes.unit_of_measurement ?? "VA"}`, "power"));
  }
  const lights = [...entities.values()].filter((s) => isLight(s) && s.state === "on").length;
  items.push(gauge(ICONS.bulb, "Lumières", lights === 0 ? "éteintes" : `${lights} allumée${lights > 1 ? "s" : ""}`, "lights"));
  // Le prochain rendez-vous d'aujourd'hui (agenda.js, étape 75).
  const next = typeof agendaGauge === "function" ? agendaGauge() : null;
  if (next) items.push(next);
  const zone = entities.get("zone.home");
  if (Array.isArray(zone?.attributes.persons)) {
    const count = zone.attributes.persons.length;
    items.push(gauge(ICONS.person, "Présents", String(count), "people"));
  }
  box.hidden = items.length === 0;
  box.replaceChildren(...items);
}
