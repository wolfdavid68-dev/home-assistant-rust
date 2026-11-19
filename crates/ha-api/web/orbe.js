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
  const wave = svg("svg", { class: "orb-wave absolute [:not(:where(.o109701))&]:[left:4px] [:not(:where(.o109702))&]:[right:4px] [:not(:where(.o109703))&]:[top:50%] [width:calc(100%_-_8px)] h-[40px] [transform:translateY(-50%)] [overflow:visible] [fill:none] [stroke:#fff] [stroke-width:2.2] [stroke-linecap:round] [stroke-linejoin:round] [vector-effect:non-scaling-stroke] [filter:drop-shadow(0_0_4px_var(--orb-a))] [opacity:0] [transition:opacity_0.3s_ease] pointer-events-none [.voice-orb[data-state=listening]_&]:[opacity:0.95]", viewBox: "0 0 100 40", preserveAspectRatio: "none", "aria-hidden": "true" });
  wave.append(svg("path", { d: "M0 20 L100 20" }));
  box.append(
    el("span", { class: "orb-glow absolute [inset:6px] [border-radius:50%] [background:var(--orb-a)] [filter:blur(22px)] [opacity:0.5] [:not(:where(.o109106))&]:[transform:scale(calc(0.92_+_var(--voice-level)_*_0.45))] [:not(:where(.o109107))&]:[transition:background_0.6s_ease,_transform_0.12s_ease-out] [:not(:where(.o109108))&]:[animation:orb-breathe_var(--orb-speed)_ease-in-out_infinite] [@media(prefers-reduced-motion:reduce)]:[:not(:where(.o110200))&]:[transform:none] [@media(prefers-reduced-motion:reduce)]:[:not(:where(.o110201))&]:[transition:background_0.3s_ease] [@media(prefers-reduced-motion:reduce)]:[:not(:where(.o110100))&]:[animation:none]!" }),
    el("span", { class: "orb-core relative w-[84px] h-[84px] overflow-hidden [border-radius:50%] [background:radial-gradient(circle_at_34%_28%,_rgb(255_255_255_/_0.75),_transparent_34%),_radial-gradient(circle_at_50%_55%,_var(--orb-a),_var(--orb-b)_72%,_color-mix(in_srgb,_var(--orb-b)_60%,_#000)_100%)] [box-shadow:inset_0_-8px_18px_rgb(0_0_0_/_0.25),_inset_0_2px_6px_rgb(255_255_255_/_0.35),_0_6px_22px_color-mix(in_srgb,_var(--orb-b)_45%,_transparent)] [transition:box-shadow_0.6s_ease] [:not(:where(.o109600)).voice-orb[data-state=speaking]_&]:[animation:orb-talk_0.9s_ease-in-out_infinite] [:not(:where(.o109900)).voice-orb[data-state=error]_&]:[animation:orb-shake_0.4s_ease-in-out_2] [@media(prefers-reduced-motion:reduce)]:[.voice-orb_&]:[animation:none]!" }, el("span", { class: "orb-shine absolute [inset:0] [border-radius:50%] [background:radial-gradient(circle_at_70%_30%,_rgb(255_244_214_/_0.85),_transparent_22%),_radial-gradient(circle_at_30%_72%,_rgb(255_225_180_/_0.6),_transparent_20%)] [opacity:0] [transition:opacity_0.4s_ease] [.voice-orb[data-state=speaking]_&]:[opacity:1] [.voice-orb[data-state=speaking]_&]:[animation:orb-shine_2.2s_ease-in-out_infinite] [@media(prefers-reduced-motion:reduce)]:[animation:none]!" }), el("span", { class: "orb-swirl absolute [inset:-20%] [background:conic-gradient(from_var(--orb-turn),_transparent_0_18%,_rgb(255_255_255_/_0.32)_26%,_transparent_36%_62%,_color-mix(in_srgb,_var(--orb-a)_70%,_#fff)_72%,_transparent_82%)] [mix-blend-mode:soft-light] [:not(:where(.o109304))&]:[animation:orb-turn_var(--orb-speed)_linear_infinite] [@media(prefers-reduced-motion:reduce)]:[:not(:where(.o110100))&]:[animation:none]!" })),
    wave,
  );
  // Une grande cible pour le doigt ; le bouton micro reste la commande
  // annoncée aux lecteurs d'écran.
  box.addEventListener("click", () => $("#assist-mic").click());
  orbDraw();
}

orbBuild();
