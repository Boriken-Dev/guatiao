// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What a library imports from `guatiao`, `guatiao/kinds` and
// `guatiao/provide`, with the shapes the Node binding gives the same names.
// Run once per context as a script; the three modules re-export from the
// object it stores under `Symbol.for('guatiao.core')`.

(() => {
  const Status = Object.freeze({
    OK: 0,
    ERR_BAD_VALUE: 7,
    ERR_ALLOC: 8,
    ERR_WRONG_KIND: 9,
    ERR_NOT_FOUND: 10,
    ERR_NULL: 11,
    ERR_GONE: 12,
    ERR_UNSUPPORTED: 13,
    ERR_BUSY: 14,
    ERR_INTERNAL: 100,
  });

  // One failing status. `status` is its code, `message` the words a
  // caller reads; a class carries its own status, so `throw new
  // BadValue('nobody to greet')` refuses a call with it.
  class GuatiaoError extends Error {
    static STATUS = Status.ERR_INTERNAL;
    constructor(first, second = '') {
      super(typeof first === 'string' ? first : second);
      this.name = new.target.name;
      this.status = typeof first === 'number' ? first : new.target.STATUS;
    }
  }
  const subclass = (name, status) => ({ [name]: class extends GuatiaoError { static STATUS = status; } })[name];
  const BadValue = subclass('BadValue', Status.ERR_BAD_VALUE);
  const AllocFailed = subclass('AllocFailed', Status.ERR_ALLOC);
  const WrongKind = subclass('WrongKind', Status.ERR_WRONG_KIND);
  const NotFound = subclass('NotFound', Status.ERR_NOT_FOUND);
  const NullArgument = subclass('NullArgument', Status.ERR_NULL);
  const Gone = subclass('Gone', Status.ERR_GONE);
  const Unsupported = subclass('Unsupported', Status.ERR_UNSUPPORTED);
  const Busy = subclass('Busy', Status.ERR_BUSY);
  const Internal = subclass('Internal', Status.ERR_INTERNAL);

  const BY_STATUS = new Map([
    [Status.ERR_BAD_VALUE, BadValue],
    [Status.ERR_ALLOC, AllocFailed],
    [Status.ERR_WRONG_KIND, WrongKind],
    [Status.ERR_NOT_FOUND, NotFound],
    [Status.ERR_NULL, NullArgument],
    [Status.ERR_GONE, Gone],
    [Status.ERR_UNSUPPORTED, Unsupported],
    [Status.ERR_BUSY, Busy],
    [Status.ERR_INTERNAL, Internal],
  ]);

  // The error a failing status is, carrying `message`.
  const errorFor = (status, message = '') => new (BY_STATUS.get(status) ?? GuatiaoError)(status, message);

  // A number whose exact text no JavaScript number holds (`1.10`, a
  // 40-digit decimal): `toString()` gives the text, arithmetic its float.
  class ExactNumber {
    #text;
    constructor(text) {
      this.#text = String(text);
    }
    toString() {
      return this.#text;
    }
    valueOf() {
      return Number(this.#text);
    }
    toJSON() {
      return this.#text;
    }
  }

  // What every provider class extends: the kinds it serves, listed in
  // `static kinds`, and the optional statics `named`, `id`, `displayName`,
  // `meta` and `available()`.
  class Provider {
    static kinds = [];
    static available() {
      return undefined;
    }
  }

  // `using` and the Node binding's disposable objects name `Symbol.dispose`.
  if (typeof Symbol.dispose !== 'symbol') {
    Object.defineProperty(Symbol, 'dispose', { value: Symbol('Symbol.dispose') });
  }

  // UTF-8 text and bytes, as Node's globals do for the library code that
  // builds a `bytes_mut` answer or reads `bytes`.
  class TextEncoder {
    get encoding() {
      return 'utf-8';
    }
    encode(input = '') {
      const text = String(input);
      const out = [];
      for (let i = 0; i < text.length; i++) {
        let c = text.codePointAt(i);
        if (c > 0xffff) i++;
        if (c >= 0xd800 && c <= 0xdfff) c = 0xfffd;
        if (c < 0x80) out.push(c);
        else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
        else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
        else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      }
      return Uint8Array.from(out);
    }
  }
  class TextDecoder {
    get encoding() {
      return 'utf-8';
    }
    decode(input) {
      let bytes;
      if (input === undefined) bytes = new Uint8Array(0);
      else if (input instanceof ArrayBuffer) bytes = new Uint8Array(input);
      else if (ArrayBuffer.isView(input)) bytes = new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      else throw new TypeError('decode takes an ArrayBuffer or a view of one');
      const points = [];
      for (let i = 0; i < bytes.length; ) {
        const lead = bytes[i];
        if (lead < 0x80) {
          points.push(lead);
          i++;
          continue;
        }
        let need;
        let point;
        let lower = 0x80;
        let upper = 0xbf;
        if (lead >= 0xc2 && lead <= 0xdf) [need, point] = [1, lead & 0x1f];
        else if (lead >= 0xe0 && lead <= 0xef) {
          [need, point] = [2, lead & 0x0f];
          if (lead === 0xe0) lower = 0xa0;
          if (lead === 0xed) upper = 0x9f;
        } else if (lead >= 0xf0 && lead <= 0xf4) {
          [need, point] = [3, lead & 0x07];
          if (lead === 0xf0) lower = 0x90;
          if (lead === 0xf4) upper = 0x8f;
        } else {
          points.push(0xfffd);
          i++;
          continue;
        }
        // A bad byte ends the sequence and is read again as the next one.
        let taken = 1;
        for (; taken <= need; taken++) {
          const next = bytes[i + taken];
          if (next === undefined || next < lower || next > upper) break;
          lower = 0x80;
          upper = 0xbf;
          point = (point << 6) | (next & 0x3f);
        }
        if (taken <= need) {
          points.push(0xfffd);
          i += taken;
          continue;
        }
        points.push(point);
        i += need + 1;
      }
      let text = '';
      for (let i = 0; i < points.length; i += 4096) text += String.fromCodePoint(...points.slice(i, i + 4096));
      return text;
    }
  }
  if (typeof globalThis.TextEncoder !== 'function') globalThis.TextEncoder = TextEncoder;
  if (typeof globalThis.TextDecoder !== 'function') globalThis.TextDecoder = TextDecoder;

  const kindFrom = (description, known) => {
    if (description === null || typeof description !== 'object' || typeof description.name !== 'string') {
      throw new TypeError('a kind description has a name');
    }
    if (!Array.isArray(description.slots)) throw new TypeError(`the kind ${description.name} has slots`);
    const camel = (name) => name.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());
    const methods = description.slots.map((slot) => camel(String(slot.name)));
    const kind = Object.freeze({
      name: description.name,
      floorHash: description.floor_hash,
      isObject: description.object === true,
      methods: Object.freeze(methods),
    });
    known[description.name] = kind;
    return kind;
  };

  // The kinds a description document holds (an object or its JSON text),
  // by name. The host gives the engine the same document; a kind that
  // differs from the host's is refused when the library opens.
  const kindsOf = (document) => {
    const parsed = typeof document === 'string' ? JSON.parse(document) : document;
    if (parsed === null || typeof parsed !== 'object' || parsed.format !== 1 || !Array.isArray(parsed.kinds)) {
      throw new TypeError('a kinds document is {"format": 1, "kinds": [...]}');
    }
    const known = Object.create(null);
    for (const description of parsed.kinds) kindFrom(description, known);
    return known;
  };
  const kindOf = (description) => kindsOf({ format: 1, kinds: [description] })[description.name];

  globalThis[Symbol.for('guatiao.core')] = {
    Status,
    GuatiaoError,
    BadValue,
    AllocFailed,
    WrongKind,
    NotFound,
    NullArgument,
    Gone,
    Unsupported,
    Busy,
    Internal,
    errorFor,
    ExactNumber,
    Provider,
    kindsOf,
    kindOf,
  };
})();
