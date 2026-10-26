// ---------------------------------------------------------------- Les cartes en verre
// Étape 99 : quand la maison répond avec un chiffre (« Salon : 19,5 °C »), une
// heure (« fini vers 11:20 ») ou une liste (« À la maison : Alice, Bob »), une
// petite carte en verre sort de l'orbe et montre la réponse en grand, lisible
// de loin. Trois cartes au plus : la plus récente en grand, deux plus petites ;
// chacune s'efface après une minute, toutes partent quand Assist se ferme.
//
// La réponse d'Assist n'est que du texte ; on le relit ici sans rien demander
// de plus au mini-PC. Le titre de la carte dit d'où vient la valeur. Les cartes
// répètent le fil de la conversation (déjà lu par les lecteurs d'écran) : elles
// sont donc cachées pour eux. Pas d'innerHTML ici non plus.
"use strict";

/** Une carte reste ce temps-là, puis s'efface. */
const CARD_LIFE_MS = 60_000;
/** Le temps du fondu de sortie (le même que dans la feuille de style). */
const CARD_FADE_MS = 400;
/** Trois cartes au plus : une grande et deux petites. */
const CARD_MAX = 3;
/** Au-delà, ce n'est plus une valeur mais une phrase : pas de carte. */
const CARD_LABEL_MAX = 48;

/** Un nombre et son unité : « 19,5 °C », « 450 W », « 72 % ». */
const CARD_NUMBER = /(-?\d+(?:[,.]\d+)?)\s?(°C|°|%|kWh|kW|W|VA|€|km\/h|mm|lx|ppm|min|h)(?![\p{L}\d])/u;
/** Une heure : « 11:20 », « 7h05 ». */
const CARD_TIME = /\b(\d{1,2})[:h](\d{2})\b/;
/** Un nombre seul : « 3 colis ». */
const CARD_BARE = /(?<![\p{L}\d])(-?\d+(?:[,.]\d+)?)(?![\p{L}\d:])/u;
/** Ces réponses confirment un ordre : rien à montrer en grand. */
const CARD_SKIP = /^(c'est fait|c’est fait|j'ai|j’ai|d'accord|ok|oui|non|pardon|désolé)\b/i;

/** La valeur à mettre en grand, et ce qui reste de la phrase autour. */
function cardFigure(value) {
  // « chauffe, consigne 21 °C, il fait 19,5 °C » : la mesure passe avant la consigne.
  const measured = /il fait (-?\d+(?:[,.]\d+)?)\s?(°C|°)/.exec(value);
  const found = measured || CARD_NUMBER.exec(value);
  if (found) return { big: found[1], unit: found[2], rest: cardRest(value, found) };
  const time = CARD_TIME.exec(value);
  if (time) return { big: `${time[1].padStart(2, "0")}:${time[2]}`, unit: "", rest: cardRest(value, time) };
  const bare = CARD_BARE.exec(value);
  if (bare) return { big: bare[1], unit: "", rest: value };
  return null;
}

/** Le reste de la phrase sans la valeur : « en marche, fini vers ». */
function cardRest(value, found) {
  const rest = `${value.slice(0, found.index)}${value.slice(found.index + found[0].length)}`;
  return rest
    .replace(/\s*,\s*,/g, ",")
    .replace(/\s{2,}/g, " ")
    .replace(/^[\s,;:–-]+|[\s,;:–-]+$/g, "")
    .replace(/\s+il fait$/, "");
}

/** « Alice, Bob et Chloé » : une liste de deux noms ou plus, sinon rien. */
function cardList(value) {
  const items = value.split(/\s*,\s*|\s+et\s+/).map((item) => item.trim()).filter(Boolean);
  if (items.length < 2 || items.length > 8 || items.some((item) => item.length > 32)) return null;
  return items;
}

/**
 * Relit une réponse d'Assist et en tire au plus trois cartes. Les réponses de
 * la maison ressemblent à « Salon : 19,5 °C ; Cuisine (Rez) : 20 °C. » ou à
 * une phrase seule ; une partie sans chiffre ni liste ne donne pas de carte.
 */
function cardsFrom(speech) {
  const text = String(speech).trim().replace(/[.!…]+$/, "");
  if (!text || CARD_SKIP.test(text)) return [];
  const found = [];
  for (const part of text.split(/\s*;\s*/)) {
    const pair = /^(.+?)\s:\s(.+)$/.exec(part);
    const label = pair ? pair[1].trim() : "Assist";
    const value = pair ? pair[2].trim() : part.trim();
    if (!value || label.length > CARD_LABEL_MAX || CARD_SKIP.test(label)) continue;
    const figure = cardFigure(value);
    // Une phrase sans « : » ne fait une carte qu'avec une vraie mesure ou une heure.
    if (figure && (pair || figure.unit || CARD_TIME.test(value))) {
      // Une phrase seule reste entière sous la valeur : coupée, elle se lirait mal.
      found.push({ label, value, ...figure, rest: pair ? figure.rest : value });
    } else if (pair) {
      const list = cardList(value);
      if (list) found.push({ label, value, chips: list });
    }
    if (found.length === CARD_MAX) break;
  }
  return found;
}

/** L'entité dont parle la carte, si son nom correspond : pour l'heure du relevé. */
function cardEntity(label) {
  if (label === "Assist" || typeof entities === "undefined") return null;
  const plain = label.replace(/\s*\([^)]*\)$/, "").toLocaleLowerCase("fr");
  for (const state of entities.values()) {
    if (nameOf(state).toLocaleLowerCase("fr") === plain) return state;
  }
  return null;
}

/** Une carte : la source en petit, la valeur en grand, le reste dessous. */
function cardElement(card) {
  const node = el("article", { class: "glass-card" }, el("p", { class: "glass-source", text: card.label }));
  if (card.chips) {
    node.classList.add("is-list");
    node.append(el("ul", { class: "glass-chips" }, ...card.chips.map((chip) => el("li", { text: chip }))));
  } else {
    const figure = el("p", { class: "glass-value" }, el("span", { class: "glass-number", text: card.big }));
    if (card.unit) figure.append(el("span", { class: "glass-unit", text: card.unit }));
    node.append(figure);
    if (card.rest) node.append(el("p", { class: "glass-sub", text: card.rest }));
  }
  const state = cardEntity(card.label);
  const when = state && typeof shortWhen === "function" ? shortWhen(state.last_updated) : "";
  if (when) node.append(el("p", { class: "glass-when", text: `relevé à ${when}` }));
  // Teinte par famille (étape 100), si elle est chargée.
  if (typeof cardFamily === "function") {
    const family = cardFamily(card, state);
    if (family) node.dataset.family = family;
  }
  return node;
}

/** Fait partir une carte en douceur. */
function cardLeave(node) {
  if (!node.isConnected || node.classList.contains("is-leaving")) return;
  clearTimeout(Number(node.dataset.timer));
  node.classList.add("is-leaving");
  setTimeout(() => node.remove(), CARD_FADE_MS);
}

/** Une réponse de la maison : ses cartes passent devant, les plus vieilles partent. */
function cardsShow(speech) {
  const box = document.getElementById("assist-cards");
  if (!box) return;
  const fresh = cardsFrom(speech);
  if (fresh.length === 0) return;
  // La première carte de la réponse devient la grande carte.
  for (const card of fresh.reverse()) {
    const node = cardElement(card);
    node.dataset.timer = String(setTimeout(() => cardLeave(node), CARD_LIFE_MS));
    box.prepend(node);
  }
  const staying = [...box.children].filter((node) => !node.classList.contains("is-leaving"));
  staying.slice(CARD_MAX).forEach(cardLeave);
}

/** Fin de la conversation : toutes les cartes partent. */
function cardsClear() {
  const box = document.getElementById("assist-cards");
  if (!box) return;
  for (const node of [...box.children]) clearTimeout(Number(node.dataset.timer));
  box.replaceChildren();
}

document.getElementById("assist")?.addEventListener("close", cardsClear);
