// ---------------------------------------------------------------- L'orbe de la voix
// Étape 98 : en haut d'Assist, une boule lumineuse montre ce que fait la
// voix, de loin, sans lire : bleu-gris quand elle attend, turquoise quand elle
// écoute (une ligne suit le niveau du micro), violet quand elle réfléchit,
// orange quand elle répond, rouge un instant quand ça n'a pas marché.
//
// voix.js lui donne l'état (setVoiceState) et le niveau du micro
// (voiceChunk) ; account.js lui signale une réponse en erreur (assistSay).
// Toucher l'orbe fait comme le bouton micro. Pas d'innerHTML ici non plus.
"use strict";

/** Le rouge d'une erreur reste ce temps-là, puis l'orbe revient au repos. */
const ORB_ERROR_MS = 2_500;
/** Les derniers niveaux du micro dessinés par la ligne. */
const ORB_POINTS = 28;

const orb = { levels: new Array(ORB_POINTS).fill(0), frame: 0, errorTimer: 0 };

function orbBox() {
  return document.getElementById("voice-orb");
}

/** L'état de la voix (voix.js) : la couleur de l'orbe suit. */
function orbState(state) {
  const box = orbBox();
  if (!box) return;
  clearTimeout(orb.errorTimer);
  box.dataset.state = state === "starting" ? "listening" : state;
  if (state !== "listening") {
    orb.levels.fill(0);
    box.style.setProperty("--voice-level", "0");
    orbDraw();
  }
}

/** Un morceau du micro : la ligne avance, le halo grossit avec la voix. */
function orbLevel(level) {
  const box = orbBox();
  if (!box) return;
  orb.levels.push(Math.min(1, level * 10));
  orb.levels.shift();
  if (!orb.frame) {
    orb.frame = requestAnimationFrame(() => {
      orb.frame = 0;
      box.style.setProperty("--voice-level", orb.levels.at(-1).toFixed(2));
      orbDraw();
    });
  }
}

/** Une réponse en erreur : rouge un instant, sauf si la voix est occupée. */
function orbError() {
  const box = orbBox();
  if (!box || !["idle", "thinking", "speaking"].includes(box.dataset.state)) return;
  box.dataset.state = "error";
  clearTimeout(orb.errorTimer);
  orb.errorTimer = setTimeout(() => {
    if (box.dataset.state === "error") box.dataset.state = "idle";
  }, ORB_ERROR_MS);
}

/** La ligne : une onde lissée, plus haute quand la voix est forte. */
function orbDraw() {
  const path = orbBox()?.querySelector(".orb-wave path");
  if (!path) return;
  const step = 100 / (ORB_POINTS - 1);
  const points = orb.levels.map((level, i) => {
    // Une pointe sur deux vers le haut, l'autre vers le bas ; les bords
    // restent calmes, pour que la ligne entre et sorte de la boule.
    const edge = Math.sin((i / (ORB_POINTS - 1)) * Math.PI);
    const amp = (2 + level * 15) * edge;
    return [i * step, 20 + (i % 2 ? amp : -amp)];
  });
  let d = `M${points[0][0].toFixed(1)} ${points[0][1].toFixed(1)}`;
  for (let i = 1; i < points.length; i += 1) {
    const [x0, y0] = points[i - 1];
    const [x1, y1] = points[i];
    d += ` Q${x0.toFixed(1)} ${y0.toFixed(1)} ${((x0 + x1) / 2).toFixed(1)} ${((y0 + y1) / 2).toFixed(1)}`;
  }
  const [xl, yl] = points.at(-1);
  d += ` L${xl.toFixed(1)} ${yl.toFixed(1)}`;
  path.setAttribute("d", d);
}

/** Construit l'orbe (une fois) : un halo, la boule, ses reflets, la ligne. */
function orbBuild() {
  const box = orbBox();
  if (!box || box.childElementCount > 0) return;
  const wave = svg("svg", { class: "orb-wave", viewBox: "0 0 100 40", preserveAspectRatio: "none", "aria-hidden": "true" });
  wave.append(svg("path", { d: "M0 20 L100 20" }));
  box.append(
    el("span", { class: "orb-glow" }),
    el("span", { class: "orb-core" }, el("span", { class: "orb-shine" }), el("span", { class: "orb-swirl" })),
    wave,
  );
  // Une grande cible pour le doigt ; le bouton micro reste la commande
  // annoncée aux lecteurs d'écran.
  box.addEventListener("click", () => $("#assist-mic").click());
  orbDraw();
}

orbBuild();
