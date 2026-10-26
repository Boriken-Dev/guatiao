// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A document to plain JavaScript and back, three ways: through a `Value`
// (`loads(...).toJS()`, `dumps(plain)`, which builds a tree), through the
// wire bytes as a tape read and written in JavaScript (`parse`, `render`),
// and through koffi callbacks (a visitor table the library calls per
// event, a walker calling the library's visitor per event).
//
//     GUATIAO_LIBRARY=<target>/release node benchmarks/node/documents.mjs [--save NAME]
//
// after `npm ci` in bindings/node and `npm run build` in its guatiao and
// guatiao-serde packages. Writes `benchmarks/results/NAME.json` in the
// runner's shape.

import assert from 'node:assert/strict';
import { writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import * as n from '../../bindings/node/guatiao/dist/ffi.js';
import * as serde from '../../bindings/node/guatiao-serde/dist/index.js';

const k = n.koffi();
const lib = k.load(serde.library().path);
const visit = lib.func('uint32_t guatiao_json_visit(guatiao_str text, uint32_t how, void *alloc, void *visitor, void *err)');
const emitWalker = lib.func('uint32_t guatiao_json_emit_walker(void *walker, uint32_t how, void *alloc, void *out)');

const P0 = k.proto('uint32_t bench_cb0(void *ctx)');
const PB = k.proto('uint32_t bench_cbb(void *ctx, bool value)');
const PS = k.proto('uint32_t bench_cbs(void *ctx, guatiao_str text)');
const PY = k.proto('uint32_t bench_cby(void *ctx, guatiao_bytes bytes)');
const PN = k.proto('uint32_t bench_cbn(void *ctx, uint64_t len)');
const PW = k.proto('uint32_t bench_walk(void *ctx, void *into)');

// guatiao_visitor: struct_size, ctx, then eleven function pointers.
const VISITOR_SIZE = 16 + 11 * 8;
const SLOT = { absent: 0, null: 1, boolean: 2, number: 3, string: 4, bytes: 5, listBegin: 6, listEnd: 7, mapBegin: 8, key: 9, mapEnd: 10 };

const text = (s) => n.textAt(BigInt(s.ptr ?? 0), Number(s.len));

// ---- parse through callbacks ---------------------------------------------

let stack = [];
let keys = [];
let key = null;
let root;

function put(v) {
  if (stack.length === 0) root = v;
  else {
    const top = stack[stack.length - 1];
    if (Array.isArray(top)) top.push(v);
    else top[key] = v;
  }
  return 0;
}
function begin(c) {
  keys.push(key);
  stack.push(c);
  return 0;
}
function end() {
  const c = stack.pop();
  key = keys.pop();
  return put(c);
}
function number(s) {
  const t = text(s);
  if (/[.eE]/.test(t)) return Number(t);
  const v = Number(t);
  return Number.isSafeInteger(v) ? v : BigInt(t);
}

const visitor = n.alloc(VISITOR_SIZE);
n.setU32(visitor, 0, VISITOR_SIZE);
const parseCallbacks = [
  [SLOT.absent, k.register(() => put(undefined), k.pointer(P0))],
  [SLOT.null, k.register(() => put(null), k.pointer(P0))],
  [SLOT.boolean, k.register((_c, b) => put(Boolean(b)), k.pointer(PB))],
  [SLOT.number, k.register((_c, s) => put(number(s)), k.pointer(PS))],
  [SLOT.string, k.register((_c, s) => put(text(s)), k.pointer(PS))],
  [SLOT.bytes, k.register((_c, b) => put(n.bytesAt(BigInt(b.ptr ?? 0), Number(b.len))), k.pointer(PY))],
  [SLOT.listBegin, k.register(() => begin([]), k.pointer(PN))],
  [SLOT.listEnd, k.register(() => end(), k.pointer(P0))],
  [SLOT.mapBegin, k.register(() => begin({}), k.pointer(PN))],
  [SLOT.key, k.register((_c, s) => ((key = text(s)), 0), k.pointer(PS))],
  [SLOT.mapEnd, k.register(() => end(), k.pointer(P0))],
];
for (const [slot, fn] of parseCallbacks) n.setPtr(visitor, 16 + slot * 8, fn);

function parseWithCallbacks(doc) {
  stack = [];
  keys = [];
  key = null;
  root = undefined;
  const status = visit(n.str(doc), 0, serde.library().alloc, visitor, null);
  if (status !== 0) throw new Error(`visit refused: ${status}`);
  return root;
}

// ---- render through callbacks --------------------------------------------

let into = 0n;
let walking;
const call = (slot, proto, ...args) => {
  const status = k.call(n.ptrAt(into, 16 + slot * 8), proto, n.ptrAt(into, 8), ...args);
  if (status !== 0) throw new Error(`the visitor refused: ${status}`);
};
function go(o) {
  if (typeof o === 'string') call(SLOT.string, PS, n.str(o));
  else if (o === null) call(SLOT.null, P0);
  else if (typeof o === 'boolean') call(SLOT.boolean, PB, o);
  else if (typeof o === 'number' || typeof o === 'bigint') call(SLOT.number, PS, n.str(String(o)));
  else if (Array.isArray(o)) {
    call(SLOT.listBegin, PN, o.length);
    for (const item of o) go(item);
    call(SLOT.listEnd, P0);
  } else {
    const ks = Object.keys(o);
    call(SLOT.mapBegin, PN, ks.length);
    for (const kk of ks) {
      call(SLOT.key, PS, n.str(kk));
      go(o[kk]);
    }
    call(SLOT.mapEnd, P0);
  }
}
const walk = k.register((_ctx, visitorAt) => {
  into = visitorAt;
  go(walking);
  return 0;
}, k.pointer(PW));
const walker = n.alloc(24);
n.setU32(walker, 0, 24);
n.setPtr(walker, 16, walk);

function renderWithCallbacks(obj) {
  walking = obj;
  const out = n.alloc(n.SIZE.value);
  try {
    n.raiseForStatus(emitWalker(walker, 0, serde.library().alloc, out));
    return n.stringText(out + BigInt(n.VALUE.payload));
  } finally {
    n.freeNode(out);
    n.release(out);
  }
}

// ---- the cases and the clock ---------------------------------------------

function config() {
  const servers = [];
  for (let i = 0; i < 50; i++) {
    servers.push({
      host: `node-${i}.example`,
      port: 8000 + i,
      weight: i / 7,
      tags: ['edge', 'eu', String(i)],
      enabled: i % 3 !== 0,
      limits: { rps: 1000 + i, burst: 2.5, note: null },
    });
  }
  return { name: 'fleet', servers };
}

function cases() {
  const floats = [];
  for (let i = 0; i < 10_000; i++) floats.push(i * 1.25 + 0.5);
  return { config: config(), floats_10k: floats, text_1mb: { blob: 'x'.repeat(1 << 20) } };
}

function time(fn, targetS, samples) {
  fn();
  let count = 1;
  for (;;) {
    const t = performance.now();
    for (let i = 0; i < count; i++) fn();
    if ((performance.now() - t) / 1000 >= targetS) break;
    count *= 2;
  }
  const out = [];
  for (let s = 0; s < samples; s++) {
    const t = performance.now();
    for (let i = 0; i < count; i++) fn();
    out.push((performance.now() - t) / count);
  }
  return out;
}

const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};

function main() {
  const args = process.argv.slice(2);
  const save = args.includes('--save') ? args[args.indexOf('--save') + 1] : undefined;
  const samples = args.includes('--samples') ? Number(args[args.indexOf('--samples') + 1]) : 15;
  const targetMs = args.includes('--target-ms') ? Number(args[args.indexOf('--target-ms') + 1]) : 20;
  const metrics = {};
  for (const [name, obj] of Object.entries(cases())) {
    const doc = serde.render(obj);
    // Every path reads and writes the same thing.
    const viaValue = serde.loads(doc);
    assert.deepEqual(viaValue.toJS(), obj);
    viaValue.close();
    assert.deepEqual(serde.parse(doc), obj);
    assert.deepEqual(parseWithCallbacks(doc), obj);
    assert.equal(serde.dumps(obj), doc);
    assert.equal(renderWithCallbacks(obj), doc);
    metrics[`${name}_parse_value`] = () => {
      const v = serde.loads(doc);
      try {
        return v.toJS();
      } finally {
        v.close();
      }
    };
    metrics[`${name}_parse_tape`] = () => serde.parse(doc);
    metrics[`${name}_parse_callbacks`] = () => parseWithCallbacks(doc);
    metrics[`${name}_render_value`] = () => serde.dumps(obj);
    metrics[`${name}_render_tape`] = () => serde.render(obj);
    metrics[`${name}_render_callbacks`] = () => renderWithCallbacks(obj);
  }
  const only = process.env.BENCH_ONLY;
  const result = {
    name: save ?? 'documents',
    metrics: {},
    raw: { processor: os.arch(), os: os.platform(), node: process.versions.node, samples_ms: {} },
  };
  for (const [name, fn] of Object.entries(metrics)) {
    if (only && !name.includes(only)) continue;
    const xs = time(fn, targetMs / 1000, samples);
    result.metrics[name] = { min_ms: Math.min(...xs), median_ms: median(xs), max_ms: Math.max(...xs) };
    result.raw.samples_ms[name] = xs;
    console.log(`${name.padEnd(32)} ${(median(xs) * 1000).toFixed(1).padStart(12)} us`);
  }
  if (save) {
    const where = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'results', `${save}.json`);
    writeFileSync(where, JSON.stringify(result, null, 2) + '\n');
    console.error(`saved ${where}`);
  }
}

main();
