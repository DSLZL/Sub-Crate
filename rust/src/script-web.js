// Generic bindings required by the locked age library; application policy is Rust.
(() => {
const {TextEncoder: HostEncoder, TextDecoder: HostDecoder, ReadableStream, WritableStream, TransformStream} = SubStoreWebUtilities;
class TextEncoder extends HostEncoder { constructor() { super(); } }
class TextDecoder extends HostDecoder {
  decode(input, options) {
    if (input !== undefined && !(input instanceof ArrayBuffer) && !ArrayBuffer.isView(input)
      && !(typeof SharedArrayBuffer === 'function' && input instanceof SharedArrayBuffer)) {
      throw new TypeError('Expected an ArrayBuffer or an ArrayBuffer view');
    }
    return super.decode(input, options);
  }
}
const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
const typedProperty = (array, key) => Object.getOwnPropertyDescriptor(typedArrayPrototype, key).get.call(array);
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
    bytes.set(JSON.parse(__hostRandomValues(length)));
    return array;
  },
};
// The pinned age and notification utilities share this body reader.
class Response {
  #used = false;
  constructor(body = null, init = {}) {
    this.status = init.status === undefined ? 200 : Number(init.status);
    if (!Number.isInteger(this.status) || this.status < 200 || this.status > 599) throw new RangeError('Invalid response status');
    this.ok = this.status >= 200 && this.status < 300;
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
}
Object.assign(globalThis, {TextEncoder, TextDecoder, ReadableStream, WritableStream, TransformStream, Response});
})();
