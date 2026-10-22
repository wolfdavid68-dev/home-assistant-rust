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
