// ---------------------------------------------------------------- Parler à la maison
// Étape 84 : un bouton micro dans Assist et sur la vue hub. La tablette
// enregistre la phrase (ecoute.js), s'arrête quand on se tait, et l'envoie
// au serveur : Whisper l'écoute sur le mini-PC, Assist la comprend, et la
// réponse est lue par la voix de la maison. Rien ne part sur Internet.
//
// Le micro n'est permis par le navigateur qu'en HTTPS (étape 79).

const VOICE_RATE = 16000;
/** Une phrase dure au plus 12 s (le serveur en accepte 15). */
const VOICE_MAX_MS = 12_000;
/** Ce silence après la phrase la termine (étape 88 : 0,9 s au lieu de 1,2 s,
 *  la réponse arrive d'autant plus tôt ; une courte hésitation passe encore). */
const VOICE_SILENCE_MS = 900;
/** Sans rien entendre pendant ce temps, on abandonne. */
const VOICE_WAIT_MS = 6_000;

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
};

async function loadVoice() {
  const { status, data } = await api("GET", "/api/conversation/voice");
  voice.listen = status === 200 && data?.listen === true;
  voice.speak = status === 200 && data?.speak === true;
  $("#assist-mic").hidden = !voice.listen;
  $("#hub-talk").hidden = !voice.listen;
}

const VOICE_LABELS = {
  idle: "Parler",
  starting: "Le micro s'ouvre…",
  listening: "J'écoute… (touche pour finir)",
  thinking: "Je réfléchis…",
  speaking: "Je réponds…",
};

function setVoiceState(state) {
  voice.state = state;
  for (const button of [$("#assist-mic"), $("#hub-talk")]) {
    button.dataset.state = state;
    button.setAttribute("aria-pressed", String(state === "listening"));
  }
  $("#assist-mic").setAttribute("aria-label", VOICE_LABELS[state]);
  const status = $("#assist-voice");
  status.hidden = state === "idle";
  status.textContent = VOICE_LABELS[state];
  if (state !== "listening") $("#assist-mic").style.setProperty("--voice-level", "0");
}

/** Ouvre le micro, ou termine la phrase s'il est déjà ouvert. */
async function voiceStart() {
  if (voice.state === "listening") {
    voiceStop();
    return;
  }
  if (voice.state !== "idle") return;
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
  else if (!voice.spoke && now - voice.startedAt > VOICE_WAIT_MS) {
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
  voiceClose();
  setVoiceState("thinking");
  const wav = wavFile(downsample(chunks, rate, VOICE_RATE), VOICE_RATE);
  let speech = "";
  try {
    const response = await fetch("/api/conversation/audio", {
      method: "POST",
      headers: { Authorization: `Bearer ${token.get()}`, "Content-Type": "audio/wav" },
      body: wav,
    });
    const data = await response.json().catch(() => ({}));
    if (response.ok) {
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
  if (speech && voice.speak && voice.player) {
    setVoiceState("speaking");
    await voiceSay(speech).catch(() => {});
  }
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
  const play = voice.player.createBufferSource();
  play.buffer = sound;
  play.connect(voice.player.destination);
  await new Promise((done) => {
    play.onended = done;
    play.start();
  });
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
// Fermer Assist ferme aussi le micro.
$("#assist").addEventListener("close", () => {
  if (voice.state === "listening" || voice.state === "starting") {
    voiceClose();
    setVoiceState("idle");
  }
});
setVoiceState("idle");
if (token.get()) loadVoice().catch(() => {});
