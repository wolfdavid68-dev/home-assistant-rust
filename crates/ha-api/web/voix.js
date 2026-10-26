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
};

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
  $("#voice-options").hidden = !voice.listen;
}

const VOICE_LABELS = {
  idle: "Parler",
  starting: "Le micro s'ouvre…",
  listening: "J'écoute… (touche pour finir)",
  following: "Je t'écoute encore… (touche l'écran pour arrêter)",
  thinking: "Je réfléchis…",
  speaking: "Je réponds…",
};

function setVoiceState(state) {
  voice.state = state;
  if (state === "idle") voice.following = false;
  const label = VOICE_LABELS[state === "listening" && voice.following ? "following" : state];
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
}

/** Ouvre le micro, ou termine la phrase s'il est déjà ouvert. `follow` :
 *  rouvert tout seul après une réponse, pour une phrase de suite. */
async function voiceStart({ follow = false } = {}) {
  if (!follow && performance.now() - voice.cancelledAt < VOICE_CANCEL_MS) return;
  if (voice.state === "listening") {
    voiceStop();
    return;
  }
  if (voice.state !== "idle") return;
  // Un toucher du bouton commence un nouvel enchaînement.
  if (!follow) voice.turns = 0;
  voice.following = follow;
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
  let speech = "";
  let more = false;
  try {
    const conversation = assistConversationId();
    const url = conversation
      ? `/api/conversation/audio?conversation_id=${encodeURIComponent(conversation)}`
      : "/api/conversation/audio";
    const response = await fetch(url, {
      method: "POST",
      headers: { Authorization: `Bearer ${token.get()}`, "Content-Type": "audio/wav" },
      body: wav,
    });
    const data = await response.json().catch(() => ({}));
    if (response.ok) {
      assistRemember(data);
      more = data.continue_conversation === true;
      if (typeof data.text === "string" && data.text) assistSay("me", data.text);
      const said = data.response?.speech?.plain?.speech;
      if (typeof said === "string") {
        speech = said;
        assistSay("home", said, data.response.response_type === "error" ? "is-error" : "");
      }
    } else {
      assistSay("home", typeof data?.error === "string" ? `${data.error}.` : "La maison n'a pas répondu.", "is-error");
    }
  } catch {
    assistSay("home", "La maison n'a pas répondu.", "is-error");
  }
  // Arrêté pendant l'attente (Assist fermé, page cachée) : la réponse
  // s'affiche, sans plus.
  if (run !== voice.run) return;
  if (speech && voice.speak && voice.player) {
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
  if (!player) return;
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
  const response = await fetch("/api/conversation/speech", {
    method: "POST",
    headers: { Authorization: `Bearer ${token.get()}`, "Content-Type": "application/json" },
    body: JSON.stringify({ text }),
  });
  if (!response.ok) return;
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
  },
  { capture: true },
);
// Page cachée (autre application, écran éteint) : tout s'arrête.
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden" && voice.state !== "idle") voiceCancel();
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
setVoiceState("idle");
if (token.get()) loadVoice().catch(() => {});
