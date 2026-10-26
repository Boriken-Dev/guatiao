# guatiao-remote

[![npm](https://img.shields.io/npm/v/guatiao-remote.svg)](https://www.npmjs.com/package/guatiao-remote)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Call the providers a [guatiao](https://github.com/Boriken-Dev/guatiao/)
server serves, from a browser page or any JavaScript program, over a
WebSocket: guatiao's remote protocol in plain TypeScript. No native
library, no WebAssembly module, no Node API.

> **Status: alpha.**

## Features

- **A page is a host** — list a server's libraries, open one, and call
  its providers through the kinds the server describes.
- **Calls typed by the kinds' descriptions** — trees cross as plain data
  (objects, arrays, strings, numbers, `Uint8Array`), 64-bit integers as a
  `bigint` past 2^53, and a lent map or buffer comes back written in place.
- **Objects both ways** — an object of the server's arrives as a
  `RemoteObject` to call; one of the page's goes as a `Served`, which the
  server may call back while a call is open.
- **Refusals are errors** — `BadValue`, `Gone`, `NotFound`, ... with the
  peer's own words; a lost link rejects every open call `Gone`.
- **One small dependency** — `@noble/hashes` for the key proof
  (HMAC-SHA256), which works where WebCrypto does not: a plain `http://`
  page.

## Installation

```bash
npm install guatiao-remote
```

ESM only. Any current browser, or Node 22 and later (its own WebSocket).

## Quick start

```ts
import { RemoteLink, Served } from 'guatiao-remote';

const link = await RemoteLink.connect('ws://192.0.2.21:7000/', { security: 'none', key });
const catalog = await link.list();
const library = catalog.libraries.find((l) => l.id === 'derived_greeter')!;
await link.open(library);
const greeter = library.providers[0]!.table('greeter')!;
console.log(await greeter.call('greet', ['web']));      // { greeting: 'hello, web' }

const heard: string[] = [];
const listener = new Served('listener', { heard: ([text]) => void heard.push(text as string) });
await greeter.call('greet_into', ['you', listener]);      // heard: ['hello, you']
link.close();
```

The server listens with `guatiao-remote-server --listen ws://ADDRESS:PORT
--security none` and a key file: the key is proved both ways, there is no
encryption, and whoever reads the page holds the key. Serve the page only
to those who may call the device. Over `wss://` the link runs server TLS
(`security: 'server-tls'`: the browser checks the server's certificate,
the page proves the key or a token inside TLS) or mutual TLS, the
default (with a client certificate the browser's user installed).

## Documentation

The protocol, the security choices and the server are in
[`docs/remote.md`](https://boriken-dev.github.io/guatiao/remote/); this
package's API is in its `AGENTS.md`.

## License

MPL 2.0.
