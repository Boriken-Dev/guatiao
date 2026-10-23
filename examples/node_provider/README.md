# Providers written in TypeScript

`node_greeter/` is a guatiao library written in TypeScript: a package
whose module, `greeters.ts`, serves the `greeter` and `counter` kinds:

- `Hello`, its one instance, serving both kinds; its `start` hands back a
  conversation, an object that tells the listener it was handed.
- `Shouter`, built per configuration from the schema `ShouterConfig`
  writes with `s`.

The kinds are loaded from their description by the module `guatiao-kind`
wrote, [`../greeter_kind/node/greeter_kind.js`](../greeter_kind/node/greeter_kind.js),
whose `.d.ts` declares each kind as an interface too. Each provider class
extends `Provider`, `implements` its kinds, so `tsc --strict` checks its
methods, and lists them in `static kinds`. The same library in plain
JavaScript is [`../node_js`](../node_js/).

The module is a library as the binding reads one: the package's `name` is
its id, its `version` the library's, the `Provider` classes it exports its
providers, and its `description`, `license` and `author` its meta.

```ts
import { Registry } from 'guatiao';
import { libraryOf } from 'guatiao/provide';

using reg = new Registry('my-host', '1.0');
reg.registerLibrary(await libraryOf(new URL('./node_greeter/greeters.ts', import.meta.url)));
```

A host in the same process, written in any language, then calls these
greeters through the registry's host services or the tables it answers.
The example host, [`../node_host`](../node_host/), does so; serving them
to a host that is not a Node process is not offered.
