// ---------------------------------------------------------------- Parler à la maison
// Étape 84 : un bouton micro dans Assist et sur la vue hub. La tablette
// enregistre la phrase (ecoute.js), s'arrête quand on se tait, et l'envoie
// au serveur : Whisper l'écoute sur le mini-PC, Assist la comprend, et la
// réponse est lue par la voix de la maison. Rien ne part sur Internet.
//
// Le micro n'est permis par le navigateur qu'en HTTPS (étape 79).
//
// Étape 96 : après la réponse dite, le micro se rouvre quelques secondes pour
// une phrase de suite, dans la même conversation (conversation_id).
//
// Étape 97 : le mot d'appel « Maison ». Si on le coche, la tablette écoute en
// continu (ecoute.js ne garde que les morceaux de parole), envoie chaque
// morceau au mini-PC, et se réveille quand la phrase commence par « Maison ».

const VOICE_RATE = 16000;
/** Une phrase dure au plus 12 s (le serveur en accepte 15). */
const VOICE_MAX_MS = 12_000;
/** Ce silence après la phrase la termine (étape 88 : 0,9 s au lieu de 1,2 s,
 *  la réponse arrive d'autant plus tôt ; une courte hésitation passe encore). */
const VOICE_SILENCE_MS = 900;
/** Sans rien entendre pendant ce temps, on abandonne. */
const VOICE_WAIT_MS = 6_000;
/** Enchaîner les phrases (étape 96) : après la réponse dite, le micro se
 *  rouvre ce temps-là pour une phrase de suite ; sans voix, il se referme
 *  sans rien envoyer. */
const VOICE_FOLLOW_MS = 6_000;
/** Phrases de suite au plus, après la première (le serveur en compte 6). */
const VOICE_FOLLOW_TURNS = 5;
/** Le réglage de cet appareil : « 0 », pas d'enchaînement. */
const VOICE_FOLLOW_KEY = "ha-voice-follow";
/** Un toucher qui arrête l'enchaînement ne rouvre pas le micro aussitôt. */
const VOICE_CANCEL_MS = 600;
/** Le mot d'appel (étape 97) : « 1 », écouté sur cet appareil (non par
 *  défaut). */
const VOICE_WAKE_KEY = "ha-voice-wake";
/** Après une conversation, l'écoute du mot reprend après ce temps : le
 *  micro de suite a pu se rouvrir entre-temps. */
const WAKE_RESUME_MS = 1_500;
/** Le service d'écoute a échoué : on réessaie après ce temps. */
const WAKE_RETRY_MS = 30_000;

const voice = {
  listen: false,
  speak: false,
  state: "idle",
  stream: null,
  /** Le son du micro : gardé d'une phrase à l'autre (en pause entre deux),
   *  avec son module d'écoute déjà chargé : le micro s'ouvre plus vite. */
  context: null,
  ready: false,
  source: null,
  node: null,
  player: null,
  chunks: [],
  rate: 48_000,
  startedAt: 0,
  heardAt: 0,
  spoke: false,
  floor: 1,
  /** Le micro rouvert tout seul pour une phrase de suite. */
  following: false,
  /** Phrases de suite déjà dites dans cet enchaînement. */
  turns: 0,
  /** La réponse en train d'être dite (pour l'arrêter d'un toucher). */
  playing: null,
  /** Augmente à chaque arrêt : une réponse attendue d'avant l'arrêt ne
   *  rouvre pas le micro. */
  run: 0,
  cancelledAt: -Infinity,
  /** Le micro ouvert par le mot d'appel (et non par un toucher). */
  woken: false,
};

/** L'écoute du mot d'appel : son propre micro et son propre module, fermés
 *  dès que la tablette écoute une phrase ou parle. */
const wake = {
  /** Le serveur sait l'écouter (le service d'écoute est installé). */
  available: false,
  word: "Maison",
  stream: null,
  context: null,
  ready: false,
  source: null,
  node: null,
  opening: false,
  /** Un morceau est parti : les suivants sont abandonnés en attendant. */
  sending: false,
  /** Pas avant ce moment (trop de morceaux, ou écoute en panne). */
  pausedUntil: 0,
  timer: 0,
  /** « refused » : micro refusé ; « failed » : pas pu s'ouvrir. */
  trouble: "",
};

/** Le mot d'appel est-il voulu sur cet appareil ? (non par défaut) */
function voiceWakeWanted() {
  try {
    return localStorage.getItem(VOICE_WAKE_KEY) === "1";
  } catch {
    return false;
  }
}

/** L'enchaînement est-il voulu sur cet appareil ? (oui par défaut) */
function voiceFollowWanted() {
  try {
    return localStorage.getItem(VOICE_FOLLOW_KEY) !== "0";
  } catch {
    return true;
  }
}

async function loadVoice() {
  const { status, data } = await api("GET", "/api/conversation/voice");
  voice.listen = status === 200 && data?.listen === true;
  voice.speak = status === 200 && data?.speak === true;
  $("#assist-mic").hidden = !voice.listen;
  $("#hub-talk").hidden = !voice.listen;
  $("#voice-orb").hidden = !voice.listen;
  $("#voice-options").hidden = !voice.listen;
  wake.available = voice.listen && data?.wake === true;
  if (typeof data?.wake_word === "string" && data.wake_word) wake.word = data.wake_word;
  $("#voice-wake-option").hidden = !wake.available;
  $("#voice-wake-word").textContent = wake.word;
  wakeUpdate();
}

const VOICE_LABELS = {
  idle: "Parler",
  starting: "Le micro s'ouvre…",
  listening: "J'écoute… (touche pour finir)",
  following: "Je t'écoute encore… (touche l'écran pour arrêter)",
  woken: "Oui ? Je t'écoute… (touche l'écran pour arrêter)",
  thinking: "Je réfléchis…",
  speaking: "Je réponds…",
};

function setVoiceState(state) {
  voice.state = state;
  if (state === "idle") {
    voice.following = false;
    voice.woken = false;
  }
  let shown = state;
  if (state === "listening" && voice.woken) shown = "woken";
  else if (state === "listening" && voice.following) shown = "following";
  const label = VOICE_LABELS[shown];
  for (const button of [$("#assist-mic"), $("#hub-talk")]) {
    button.dataset.state = state;
    button.setAttribute("aria-pressed", String(state === "listening"));
    // Le signe visible de l'écoute de suite : un anneau qui bat.
    button.toggleAttribute("data-follow", state === "listening" && voice.following);
  }
  $("#assist-mic").setAttribute("aria-label", label);
  const status = $("#assist-voice");
  status.hidden = state === "idle";
  status.textContent = label;
  if (state !== "listening") $("#assist-mic").style.setProperty("--voice-level", "0");
  // L'orbe en haut d'Assist prend la couleur de l'état (orbe.js, étape 98).
  if (typeof orbState === "function") orbState(state);
  // Le mot d'appel se tait dès que la tablette écoute une phrase ou parle,
  // et reprend un peu après.
  clearTimeout(wake.timer);
  if (state === "idle") wake.timer = setTimeout(wakeUpdate, WAKE_RESUME_MS);
  else wakeUpdate();
}

/** Ouvre le micro, ou termine la phrase s'il est déjà ouvert. `follow` :
 *  rouvert tout seul après une réponse, pour une phrase de suite ; `woken` :
 *  ouvert par le mot d'appel. */
async function voiceStart({ follow = false, woken = false } = {}) {
  if (!follow && !woken && performance.now() - voice.cancelledAt < VOICE_CANCEL_MS) return;
  if (voice.state === "listening") {
    voiceStop();
    return;
  }
  if (voice.state !== "idle") return;
  // Un toucher du bouton commence un nouvel enchaînement.
  if (!follow) voice.turns = 0;
  // Ouvert sans toucher : sans voix, il se referme sans rien dire.
  voice.following = follow || woken;
  voice.woken = woken;
  if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia || !window.AudioWorkletNode) {
    assistSay(
      "home",
      "Le micro ne marche qu'en HTTPS : ouvre le tableau de bord par son adresse https (voir le guide du HTTPS).",
      "is-error",
    );
    return;
  }
  setVoiceState("starting");
  // La voix de la réponse : préparée maintenant, pendant le toucher, sinon
  // le navigateur refuserait de jouer le son plus tard.
  try {
    voice.player ??= new AudioContext();
    voice.player.resume().catch(() => {});
  } catch {
    voice.player = null;
  }
  try {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true },
    });
    voice.stream = stream;
    voice.context ??= new AudioContext();
    const context = voice.context;
    await context.resume();
    if (!voice.ready) {
      await context.audioWorklet.addModule("/ecoute.js");
      voice.ready = true;
    }
    // Assist fermé pendant que le micro s'ouvrait : on n'écoute pas.
    if (voice.state !== "starting" || voice.stream !== stream) {
      if (voice.stream === stream) voiceClose();
      return;
    }
    const source = context.createMediaStreamSource(stream);
    const node = new AudioWorkletNode(context, "ecoute", { numberOfOutputs: 0 });
    node.port.onmessage = (event) => voiceChunk(event.data);
    source.connect(node);
    voice.source = source;
    voice.node = node;
    Object.assign(voice, {
      chunks: [],
      rate: context.sampleRate,
      startedAt: performance.now(),
      heardAt: 0,
      spoke: false,
      floor: 1,
    });
    setVoiceState("listening");
  } catch (error) {
    voiceClose();
    if (voice.state !== "starting") return;
    setVoiceState("idle");
    assistSay(
      "home",
      error?.name === "NotAllowedError"
        ? "Le micro est refusé : autorise-le pour cette page dans les réglages du navigateur."
        : "Le micro n'a pas pu s'ouvrir.",
      "is-error",
    );
  }
}

/** Un paquet de son : on le garde, et on regarde si la phrase est finie. */
function voiceChunk(samples) {
  if (voice.state !== "listening") return;
  voice.chunks.push(samples);
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  const level = Math.sqrt(sum / samples.length);
  const now = performance.now();
  // Le bruit de fond de la pièce : le niveau le plus bas entendu. La voix
  // est ce qui le dépasse nettement.
  voice.floor = Math.min(voice.floor, Math.max(level, 0.002));
  if (level > Math.max(0.015, voice.floor * 4)) {
    voice.heardAt = now;
    voice.spoke = true;
  }
  $("#assist-mic").style.setProperty("--voice-level", Math.min(1, level * 10).toFixed(2));
  if (typeof orbLevel === "function") orbLevel(level);
  if (now - voice.startedAt > VOICE_MAX_MS) voiceStop();
  else if (voice.spoke && now - voice.heardAt > VOICE_SILENCE_MS) voiceStop();
  else if (!voice.spoke && voice.following && now - voice.startedAt > VOICE_FOLLOW_MS) {
    // Personne n'a enchaîné : on referme, sans rien envoyer ni rien dire.
    voiceClose();
    setVoiceState("idle");
  } else if (!voice.spoke && now - voice.startedAt > VOICE_WAIT_MS) {
    voiceClose();
    setVoiceState("idle");
    assistSay("home", "Je n'ai rien entendu. Touche le micro, puis parle.", "is-error");
  }
}

/** Ferme le micro (le voyant du navigateur s'éteint). Le son reste prêt, en
 *  pause, pour la phrase suivante. */
function voiceClose() {
  for (const track of voice.stream?.getTracks() ?? []) track.stop();
  voice.stream = null;
  voice.source?.disconnect();
  voice.node?.port.close();
  voice.source = null;
  voice.node = null;
  voice.context?.suspend().catch(() => {});
}

/** La phrase est finie : elle part au serveur, la réponse revient. */
async function voiceStop() {
  if (voice.state !== "listening") return;
  const { chunks, rate } = voice;
  const run = voice.run;
  voiceClose();
  setVoiceState("thinking");
  const wav = wavFile(downsample(chunks, rate, VOICE_RATE), VOICE_RATE);
  let answer = null;
  try {
    const conversation = assistConversationId();
    const url = conversation
      ? `/api/conversation/audio?conversation_id=${encodeURIComponent(conversation)}`
      : "/api/conversation/audio";
    const response = await authFetch(url, { method: "POST", headers: { "Content-Type": "audio/wav" }, body: wav });
    if (!response) throw new Error("réseau");
    const data = await response.json().catch(() => ({}));
    if (response.ok) answer = data;
    else assistSay("home", typeof data?.error === "string" ? `${data.error}.` : "La maison n'a pas répondu.", "is-error");
  } catch {
    assistSay("home", "La maison n'a pas répondu.", "is-error");
  }
  if (answer) await voiceAnswer(answer, run);
  else if (run === voice.run) setVoiceState("idle");
}

/** La réponse de la maison : affichée, dite, puis peut-être une phrase de
 *  suite (étape 96). */
async function voiceAnswer(data, run) {
  assistRemember(data);
  const more = data.continue_conversation === true;
  if (typeof data.text === "string" && data.text) assistSay("me", data.text);
  let speech = "";
  const said = data.response?.speech?.plain?.speech;
  if (typeof said === "string") {
    speech = said;
    assistSay("home", said, data.response.response_type === "error" ? "is-error" : "");
  }
  // Arrêté pendant l'attente (Assist fermé, page cachée) : la réponse
  // s'affiche, sans plus.
  if (run !== voice.run) return;
  // (Un son que le navigateur n'a pas encore permis ne finirait jamais.)
  if (speech && voice.speak && voice.player?.state === "running") {
    setVoiceState("speaking");
    await voiceSay(speech).catch(() => {});
    if (run !== voice.run) return;
  }
  setVoiceState("idle");
  // Enchaîner : jamais pendant que la tablette parle (la réponse est finie),
  // seulement Assist ouvert et la page visible, et au plus quelques phrases.
  if (
    more &&
    voiceFollowWanted() &&
    voice.turns < VOICE_FOLLOW_TURNS &&
    $("#assist").open &&
    document.visibilityState === "visible"
  ) {
    voice.turns += 1;
    await voiceBeep();
    if (run !== voice.run || voice.state !== "idle" || !$("#assist").open) return;
    voiceStart({ follow: true });
  }
}

/** Un petit son doux, sur la tablette seulement : le micro se rouvre. */
async function voiceBeep() {
  const player = voice.player;
  if (player?.state !== "running") return;
  try {
    const tone = player.createOscillator();
    const gain = player.createGain();
    tone.frequency.value = 880;
    gain.gain.setValueAtTime(0.0001, player.currentTime);
    gain.gain.exponentialRampToValueAtTime(0.08, player.currentTime + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, player.currentTime + 0.15);
    tone.connect(gain).connect(player.destination);
    await new Promise((done) => {
      tone.onended = done;
      tone.start();
      tone.stop(player.currentTime + 0.16);
    });
  } catch {
    // Pas de son : le signe visible suffit.
  }
}

/** Arrête tout : la réponse dite, le micro, l'enchaînement. */
function voiceCancel() {
  voice.run += 1;
  voice.cancelledAt = performance.now();
  try {
    voice.playing?.stop();
  } catch {
    // Déjà finie.
  }
  voice.playing = null;
  voiceClose();
  setVoiceState("idle");
}

/** La réponse, dite par la voix de la maison, sur la tablette. */
async function voiceSay(text) {
  const response = await authFetch("/api/conversation/speech", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text }),
  });
  if (!response?.ok) return;
  const sound = await voice.player.decodeAudioData(await response.arrayBuffer());
  if (voice.state !== "speaking") return;
  const play = voice.player.createBufferSource();
  play.buffer = sound;
  play.connect(voice.player.destination);
  voice.playing = play;
  await new Promise((done) => {
    play.onended = done;
    play.start();
  });
  if (voice.playing === play) voice.playing = null;
}

/** Le son du micro, ramené à 16 000 Hz (la moyenne de chaque tranche). */
function downsample(chunks, from, to) {
  const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const input = new Float32Array(total);
  let at = 0;
  for (const chunk of chunks) {
    input.set(chunk, at);
    at += chunk.length;
  }
  if (from === to) return input;
  const ratio = from / to;
  const output = new Float32Array(Math.floor(total / ratio));
  for (let i = 0; i < output.length; i += 1) {
    const start = Math.floor(i * ratio);
    const end = Math.min(total, Math.max(start + 1, Math.floor((i + 1) * ratio)));
    let sum = 0;
    for (let j = start; j < end; j += 1) sum += input[j];
    output[i] = sum / (end - start);
  }
  return output;
}

/** Un fichier WAV : 16 bits, mono. */
function wavFile(samples, rate) {
  const buffer = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(buffer);
  const text = (offset, value) => {
    for (let i = 0; i < value.length; i += 1) view.setUint8(offset + i, value.charCodeAt(i));
  };
  text(0, "RIFF");
  view.setUint32(4, 36 + samples.length * 2, true);
  text(8, "WAVEfmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, "data");
  view.setUint32(40, samples.length * 2, true);
  for (let i = 0; i < samples.length; i += 1) {
    const sample = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(44 + i * 2, sample < 0 ? sample * 0x8000 : sample * 0x7fff, true);
  }
  return new Blob([buffer], { type: "audio/wav" });
}

// ---------------------------------------------------------------- Le mot d'appel
// Étape 97. Ce qui est dit près de la tablette part en morceaux vers le
// mini-PC (jamais sur Internet), qui ne garde rien : sans le mot, la réponse
// est seulement « non ». Un signe reste affiché tant que c'est coché.

/** Écouter le mot maintenant ? Coché, connecté, page visible, et la tablette
 *  ni n'écoute une phrase ni ne parle. */
function wakeShouldListen() {
  return (
    wake.available &&
    voiceWakeWanted() &&
    Boolean(token.get()) &&
    document.visibilityState === "visible" &&
    voice.state === "idle" &&
    performance.now() >= wake.pausedUntil
  );
}

/** Ouvre ou ferme l'écoute du mot selon le moment, puis met le signe à jour. */
function wakeUpdate() {
  if (wakeShouldListen()) wakeOpen();
  else wakeClose();
  wakeShow();
}

async function wakeOpen() {
  if (wake.stream || wake.opening) return;
  if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia || !window.AudioWorkletNode) {
    wake.trouble = "failed";
    return;
  }
  wake.opening = true;
  try {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true },
    });
    if (!wake.context) {
      wake.context = new AudioContext();
      wake.context.onstatechange = wakeShow;
    }
    const context = wake.context;
    if (!wake.ready) {
      await context.audioWorklet.addModule("/ecoute.js");
      wake.ready = true;
    }
    // Sans toucher depuis l'ouverture de la page, le navigateur peut garder
    // le son en pause : le signe demande alors un toucher.
    context.resume().catch(() => {});
    if (!wakeShouldListen()) {
      for (const track of stream.getTracks()) track.stop();
      return;
    }
    const source = context.createMediaStreamSource(stream);
    const node = new AudioWorkletNode(context, "mot-appel", { numberOfOutputs: 0 });
    node.port.onmessage = (event) => wakeHeard(event.data);
    source.connect(node);
    Object.assign(wake, { stream, source, node, trouble: "" });
  } catch (error) {
    wake.trouble = error?.name === "NotAllowedError" ? "refused" : "failed";
  } finally {
    wake.opening = false;
    wakeShow();
  }
}

/** Ferme le micro du mot d'appel (le voyant du navigateur s'éteint). */
function wakeClose() {
  if (!wake.stream) return;
  for (const track of wake.stream.getTracks()) track.stop();
  wake.source?.disconnect();
  wake.node?.port.close();
  Object.assign(wake, { stream: null, source: null, node: null });
  wake.context?.suspend().catch(() => {});
}

/** Le signe visible : à l'écoute, en pause, ou ce qui manque. */
function wakeShow() {
  const sign = $("#wake-sign");
  const wanted = wake.available && voiceWakeWanted() && Boolean(token.get());
  sign.hidden = !wanted;
  if (!wanted) return;
  let state = "paused";
  let text = `« ${wake.word} » en pause`;
  if (wake.trouble === "refused") {
    state = "trouble";
    text = `Micro refusé : « ${wake.word} » n'est pas écouté`;
  } else if (wake.trouble === "failed") {
    state = "trouble";
    text = `« ${wake.word} » ne peut pas être écouté ici`;
  } else if (wake.stream && wake.context?.state !== "running") {
    state = "touch";
    text = `Touche l'écran pour écouter « ${wake.word} »`;
  } else if (wake.stream) {
    state = "on";
    text = `À l'écoute de « ${wake.word} »`;
  }
  sign.dataset.state = state;
  $("#wake-sign-text").textContent = text;
}

/** Un morceau de parole : il part au mini-PC, qui dit si c'était le mot. */
async function wakeHeard(samples) {
  // Un seul envoi à la fois : les morceaux d'ici là sont abandonnés.
  if (wake.sending || !wake.stream || !wakeShouldListen()) return;
  wake.sending = true;
  const run = voice.run;
  try {
    const wav = wavFile(downsample([samples], wake.context.sampleRate, VOICE_RATE), VOICE_RATE);
    const response = await authFetch("/api/conversation/wake", { method: "POST", headers: { "Content-Type": "audio/wav" }, body: wav });
    // Le réseau a manqué : ce morceau est perdu, le suivant passera.
    if (!response) return;
    if (response.status === 429 || response.status >= 500) {
      // Trop de morceaux, ou l'écoute en panne : une pause, puis on reprend.
      const wait = Number(response.headers.get("Retry-After")) * 1000 || WAKE_RETRY_MS;
      wake.pausedUntil = performance.now() + wait;
      wakeUpdate();
      setTimeout(wakeUpdate, wait + 100);
      return;
    }
    if (!response.ok) return;
    const data = await response.json().catch(() => ({}));
    if (data.wake !== true || run !== voice.run || !wakeShouldListen()) return;
    // C'est le mot : la tablette se réveille, Assist s'ouvre.
    if (typeof saver !== "undefined" && saver.on) hideSaver();
    if (!$("#assist").open) $("#assist").showModal();
    if (data.response) {
      // « Maison, allume la cuisine » : déjà obéi, on dit la réponse.
      voice.turns = 0;
      await voiceAnswer(data, run);
    } else {
      // « Maison » seul : un petit son, et le micro s'ouvre.
      await voiceBeep();
      if (run === voice.run && voice.state === "idle") voiceStart({ woken: true });
    }
  } catch {
    // Le réseau a manqué : ce morceau est perdu, le suivant passera.
  } finally {
    wake.sending = false;
  }
}

/** Le premier toucher permet le son : l'écoute du mot et la voix démarrent. */
function wakeUnlock() {
  if (wake.context && wake.context.state !== "running" && wake.stream) {
    wake.context.resume().then(wakeShow, () => {});
  }
  if (wake.available && voiceWakeWanted() && !voice.player) {
    try {
      voice.player = new AudioContext();
    } catch {
      voice.player = null;
    }
  }
  if (voice.player?.state === "suspended") voice.player.resume().catch(() => {});
}

$("#assist-mic").replaceChildren(svgIcon(ICONS.mic));
hubButton("#hub-talk", ICONS.mic, "Parler à la maison");
$("#assist-mic").addEventListener("click", () => voiceStart());
$("#hub-talk").addEventListener("click", () => {
  if (!$("#assist").open) $("#assist").showModal();
  voiceStart();
});
// Fermer Assist ferme aussi le micro, et arrête la réponse et l'enchaînement.
$("#assist").addEventListener("close", () => {
  if (["listening", "starting", "speaking"].includes(voice.state)) voiceCancel();
});
// Toucher l'écran, ou le bouton, arrête l'écoute de suite et la réponse dite.
document.addEventListener(
  "pointerdown",
  () => {
    if ((voice.state === "listening" && voice.following) || voice.state === "speaking") voiceCancel();
    wakeUnlock();
  },
  { capture: true },
);
// Page cachée (autre application, écran éteint) : tout s'arrête, le mot
// d'appel aussi ; il reprend quand la page revient.
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden" && voice.state !== "idle") voiceCancel();
  wakeUpdate();
});
// Le réglage de cet appareil, à côté des autres réglages de la tablette.
$("#voice-follow").checked = voiceFollowWanted();
$("#voice-follow").addEventListener("change", (event) => {
  try {
    if (event.target.checked) localStorage.removeItem(VOICE_FOLLOW_KEY);
    else localStorage.setItem(VOICE_FOLLOW_KEY, "0");
  } catch {
    // Stockage refusé : le réglage vaut pour cette visite.
  }
});
$("#voice-wake").checked = voiceWakeWanted();
$("#voice-wake").addEventListener("change", (event) => {
  try {
    if (event.target.checked) localStorage.setItem(VOICE_WAKE_KEY, "1");
    else localStorage.removeItem(VOICE_WAKE_KEY);
  } catch {
    // Stockage refusé : le mot d'appel reste coupé.
  }
  wake.trouble = "";
  wake.pausedUntil = 0;
  // Le toucher de la case permet aussi le son.
  wakeUpdate();
  wakeUnlock();
});
$("#wake-sign-icon").replaceChildren(svgIcon(ICONS.mic));
setVoiceState("idle");
if (token.get()) loadVoice().catch(() => {});
