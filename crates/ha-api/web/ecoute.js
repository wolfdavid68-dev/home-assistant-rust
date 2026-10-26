// Le micro de la tablette (étape 84). Ce petit programme tourne à part,
// dans le fil audio du navigateur (un « AudioWorklet ») : il reçoit le son du
// micro par paquets de 128 échantillons, les regroupe, et les envoie à
// voix.js, qui décide quand la phrase est finie.

const PACKET = 2048;

class Ecoute extends AudioWorkletProcessor {
  constructor() {
    super();
    this.buffer = new Float32Array(PACKET);
    this.filled = 0;
  }

  process(inputs) {
    const channel = inputs[0]?.[0];
    if (channel) {
      let read = 0;
      while (read < channel.length) {
        const take = Math.min(channel.length - read, PACKET - this.filled);
        this.buffer.set(channel.subarray(read, read + take), this.filled);
        this.filled += take;
        read += take;
        if (this.filled === PACKET) {
          this.port.postMessage(this.buffer);
          this.buffer = new Float32Array(PACKET);
          this.filled = 0;
        }
      }
    }
    return true;
  }
}

registerProcessor("ecoute", Ecoute);

// ---------------------------------------------------------------- Le mot d'appel
// Étape 97 : quand « Écouter le mot « Maison » » est coché, ce second petit
// programme écoute en continu et ne garde que les **morceaux de parole** :
// il compare l'énergie du son au bruit de fond de la pièce (qu'il apprend
// tout seul), garde 300 ms d'avance pour ne pas couper le début du mot, et
// envoie à voix.js un morceau de 0,4 à 6 s quand on se tait. Le silence et
// les bruits courts ne partent jamais.

/** Une tranche d'analyse : 20 ms. */
const FRAME_MS = 20;
/** Gardé avant le début de la parole. */
const BEFORE_MS = 300;
/** Ce silence termine un morceau (une courte pause entre « Maison » et la
 *  commande passe encore). */
const END_MS = 700;
/** Un morceau plus court n'est pas de la parole (une porte, un clic). */
const MIN_MS = 400;
/** Un morceau ne dépasse pas ce temps (le serveur en accepte 6,5 s). */
const MAX_MS = 6_000;
/** La parole commence après ce nombre de tranches fortes à la suite. */
const START_FRAMES = 3;

class MotAppel extends AudioWorkletProcessor {
  constructor() {
    super();
    this.frameLength = Math.round((sampleRate * FRAME_MS) / 1000);
    this.frame = new Float32Array(this.frameLength);
    this.filled = 0;
    this.maxBefore = Math.ceil(BEFORE_MS / FRAME_MS);
    this.endFrames = Math.ceil(END_MS / FRAME_MS);
    this.minFrames = Math.ceil(MIN_MS / FRAME_MS);
    this.maxFrames = Math.floor(MAX_MS / FRAME_MS);
    /** Le bruit de fond, appris : il descend vite, remonte lentement. */
    this.floor = 0.01;
    this.port.onmessage = () => this.reset();
    this.reset();
  }

  /** On oublie le morceau en cours (la tablette parle, ou écoute déjà). */
  reset() {
    this.before = [];
    this.speech = null;
    this.loud = 0;
    this.quiet = 0;
    this.startAt = 0;
  }

  process(inputs) {
    const channel = inputs[0]?.[0];
    if (channel) {
      let read = 0;
      while (read < channel.length) {
        const take = Math.min(channel.length - read, this.frameLength - this.filled);
        this.frame.set(channel.subarray(read, read + take), this.filled);
        this.filled += take;
        read += take;
        if (this.filled === this.frameLength) {
          this.analyse(this.frame);
          this.frame = new Float32Array(this.frameLength);
          this.filled = 0;
        }
      }
    }
    return true;
  }

  analyse(frame) {
    let sum = 0;
    for (const sample of frame) sum += sample * sample;
    const level = Math.sqrt(sum / frame.length);
    const loud = level > Math.max(0.012, this.floor * 3);
    if (!this.speech) {
      // Le bruit de fond n'apprend que hors de la parole.
      const rate = level < this.floor ? 0.1 : 0.01;
      this.floor = Math.max(0.0005, this.floor + (level - this.floor) * rate);
      this.before.push(frame);
      this.loud = loud ? this.loud + 1 : 0;
      if (this.loud >= START_FRAMES) {
        this.speech = this.before;
        this.before = [];
        this.quiet = 0;
        // Où commence la parole (après les 300 ms gardées d'avance).
        this.startAt = this.speech.length - START_FRAMES;
      } else if (this.before.length > this.maxBefore + START_FRAMES) {
        this.before.shift();
      }
      return;
    }
    this.speech.push(frame);
    this.quiet = loud ? 0 : this.quiet + 1;
    if (this.quiet >= this.endFrames || this.speech.length >= this.maxFrames) {
      const spoken = this.speech.length - this.quiet - this.startAt;
      if (spoken >= this.minFrames) this.send(this.speech);
      this.reset();
    }
  }

  send(frames) {
    const samples = new Float32Array(frames.length * this.frameLength);
    frames.forEach((frame, at) => samples.set(frame, at * this.frameLength));
    this.port.postMessage(samples, [samples.buffer]);
  }
}

registerProcessor("mot-appel", MotAppel);
