// Contained M1 globals; Node's real Web APIs remain the independent oracle.
import {TextEncoder as Encoder, TextDecoder} from '@kayahr/text-encoding/no-encodings';
import {ReadableStream, WritableStream, TransformStream} from 'web-streams-polyfill';

// The library also offers legacy encoders; the Web TextEncoder is always UTF-8.
class TextEncoder extends Encoder { constructor() { super(); } }
Object.assign(globalThis, {TextEncoder, TextDecoder, ReadableStream, WritableStream, TransformStream});
const typedArray = Object.getPrototypeOf(Uint8Array.prototype);
const typedProperty = (array, name) => Object.getOwnPropertyDescriptor(typedArray, name).get.call(array);

globalThis.crypto = {
  getRandomValues(array) {
    const types = ['Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array',
      'Int32Array', 'Uint32Array', 'BigInt64Array', 'BigUint64Array'];
    if (!ArrayBuffer.isView(array) || !types.includes(typedProperty(array, Symbol.toStringTag))) {
      const error = new Error('Expected an integer TypedArray'); error.name = 'TypeMismatchError'; throw error;
    }
    const length = typedProperty(array, 'byteLength');
    if (length > 65536) {
      const error = new Error('Random values exceed 65536 bytes'); error.name = 'QuotaExceededError'; throw error;
    }
    const bytes = new Uint8Array(typedProperty(array, 'buffer'), typedProperty(array, 'byteOffset'), length);
    bytes.set(JSON.parse(__hostRandomValues(bytes.length)));
    return array;
  },
};

// shortcut: Response only consumes bodies for M1 age; full Fetch belongs to HG17.
globalThis.Response = class Response {
  #used = false;
  constructor(body = null) {
    this.body = body === null || body instanceof ReadableStream ? body : new ReadableStream({
      start(controller) {
        controller.enqueue(typeof body === 'string' ? new TextEncoder().encode(body)
          : ArrayBuffer.isView(body) ? new Uint8Array(body.buffer, body.byteOffset, body.byteLength).slice()
          : body instanceof ArrayBuffer ? new Uint8Array(body).slice()
          : new TextEncoder().encode(String(body)));
        controller.close();
      },
    });
  }
  get bodyUsed() { return this.#used; }
  async arrayBuffer() {
    if (this.#used || this.body?.locked) throw new TypeError('Body is unusable');
    if (!this.body) return new ArrayBuffer(0);
    this.#used = true;
    const reader = this.body.getReader(), chunks = [];
    let length = 0;
    try {
      while (true) {
        const {done, value} = await reader.read();
        if (done) break;
        if (!(value instanceof Uint8Array)) throw new TypeError('Expected Uint8Array stream chunk');
        chunks.push(value); length += value.length;
      }
    } finally { reader.releaseLock(); }
    const bytes = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
    return bytes.buffer;
  }
  async text() { return new TextDecoder().decode(await this.arrayBuffer()); }
  async json() { return JSON.parse(await this.text()); }
};
