// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A plugin host written in TypeScript.
//
// Scans the directories it is given for libraries declaring the `greeter`
// kind, whatever language they were written in, and calls each greeter
// through the kind's table: a map back, a map lent and changed in place, a
// map handed over, an object handed in and one handed back, an object lent
// for one call. A provider that builds instances from a configuration is
// given one its schema refuses, then one it accepts. Everything it opened
// is closed before it exits, and it prints what it did.
//
//     node host.ts DIR [DIR ...] [--node-library MODULE ...]
//                  [--python-source BRIDGE --python-dir DIR ... --python-path DIR ...]
//
// `--node-library` registers an ES module of provider classes (a file of
// a package) as a library; `--python-source` adds the guatiao-py bridge
// as a source of the Python libraries under each `--python-dir`.

import path from 'node:path';
import { parseArgs } from 'node:util';

import { GuatiaoError, Registry, Value } from 'guatiao';
import { FloorMismatch } from 'guatiao';
import { type Remote, liveObjects } from 'guatiao/kinds';
import { type Library, libraryOf } from 'guatiao/provide';

import { Greeter } from '../greeter_kind/node/greeter_kind.ts';

type Say = (line: string) => void;

/** A listener: what a conversation tells, kept. */
class Ear {
  readonly heardSoFar: string[] = [];
  heard(what: string): void {
    this.heardSoFar.push(what);
  }
}

/** A configuration naming every required text field of `schema`. */
function configFor(schema: Record<string, unknown>, fill: string): Record<string, string> {
  const properties = (schema.properties ?? {}) as Record<string, { type?: string }>;
  const required = (schema.required ?? []) as string[];
  return Object.fromEntries(required.filter((name) => properties[name]?.type === 'string').map((name) => [name, fill]));
}

const show = (value: unknown): string => JSON.stringify(value, (_k, v: unknown) => (typeof v === 'bigint' ? Number(v) : v));

/** Every slot of the greeter kind this table carries. */
function exercise(greeter: Remote<typeof Greeter>, say: Say): void {
  const answer = greeter.greet('ana');
  say(`greet: ${show(answer.toJS())}`);
  answer.close();
  try {
    greeter.greet('').close();
  } catch (e) {
    if (!(e instanceof GuatiaoError)) throw e;
    say(`greet refused: ${e.name}: ${e.message}`);
  }
  if (greeter.has('shout')) say(`shout: ${greeter.shout('ana')}`);
  if (greeter.has('check')) {
    const config = new Value({ name: 'bo' });
    const usable = greeter.check(config);
    say(`check: ${usable}, lent map now ${show(config.toJS())}`);
    config.close();
  }
  if (greeter.has('greetFrom')) {
    const answer = greeter.greetFrom({ name: 'cy' });
    say(`greetFrom: ${show(answer.toJS())}`);
    answer.close();
  }
  if (greeter.has('greetInto')) {
    const ear = new Ear();
    greeter.greetInto('dee', ear);
    say(`greetInto: the lent listener heard ${show(ear.heardSoFar)}`);
  }
  if (greeter.has('start')) {
    const ear = new Ear();
    let conversation;
    try {
      conversation = greeter.start(ear);
    } catch (e) {
      if (!(e instanceof GuatiaoError)) throw e;
      say(`start refused: ${e.message}`);
    }
    if (conversation !== undefined) {
      try {
        conversation.say('hi');
        conversation.say('again');
        const transcript = new Uint8Array(64);
        const n = conversation.read(transcript);
        say(
          `conversation: ${conversation.turns()} turns, ` +
            `transcript ${show(Buffer.from(transcript.subarray(0, Number(n))).toString())}, ` +
            `listener heard ${show(ear.heardSoFar)}`,
        );
      } finally {
        conversation.close();
      }
    }
  }
}

async function main(): Promise<number> {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      'node-library': { type: 'string', multiple: true, default: [] },
      'python-source': { type: 'string' },
      'python-dir': { type: 'string', multiple: true, default: [] },
      'python-path': { type: 'string', multiple: true, default: [] },
    },
  });

  const reg = new Registry('node-host', '1.0');
  const built: Library[] = [];
  try {
    await host(reg, built, positionals, values);
  } finally {
    reg.close();
    // What this host built for itself, once no registry can call it.
    for (const library of built) library.close();
  }
  console.log(`objects a library can still reach: ${liveObjects()}`);
  return 0;
}

async function host(
  reg: Registry,
  built: Library[],
  positionals: string[],
  values: { 'node-library': string[]; 'python-source'?: string | undefined; 'python-dir': string[]; 'python-path': string[] },
): Promise<void> {
  for (const dir of positionals) {
    const report = reg.scanDir(dir, { rules: 'kind=greeter' }) as { loaded: string[] };
    for (const loaded of report.loaded) console.log(`loaded ${path.basename(loaded)}`);
  }
  for (const module of values['node-library']) {
    const library = await libraryOf(path.resolve(module));
    built.push(library);
    reg.registerLibrary(library);
    console.log(`registered ${library.id} ${library.version} from Node`);
  }
  if (values['python-source'] !== undefined) {
    reg.addSourceFile(values['python-source'], { dirs: values['python-dir'], paths: values['python-path'] });
    const changed = reg.refresh();
    for (const served of changed.loaded) console.log(`served ${String(served.id)} from Python`);
    for (const refused of changed.refused) console.log(`refused ${String(refused.id ?? '')}: ${String(refused.why)}`);
  }

  for (const provider of reg.providers('greeter')) {
    const key = String(provider.key);
    console.log(`\n${key} (${String(provider.library)} ${String(provider.version)})`);
    const say: Say = (line) => console.log(`  ${line}`);
    let greeter: Remote<typeof Greeter>;
    try {
      greeter = reg.bind(key, Greeter);
    } catch (e) {
      if (!(e instanceof FloorMismatch)) throw e;
      say(`not this greeter kind: ${e.message}`);
      continue;
    }
    if (!provider.has_config) {
      exercise(greeter, say);
      continue;
    }
    // Built from a configuration: one the schema refuses, then one it
    // accepts. The registry checks it before the provider sees it.
    const schema = reg.providerConfig(key)?.toJS<Record<string, unknown>>() ?? {};
    try {
      reg.create(key, {}).close();
    } catch (e) {
      if (!(e instanceof GuatiaoError)) throw e;
      say(`an empty configuration is refused: ${e.message}`);
    }
    const config = configFor(schema, 'hey');
    const instance = reg.create(key, config);
    try {
      say(`instance from ${show(config)}`);
      exercise(instance.bind(Greeter), say);
    } finally {
      instance.close();
    }
  }

  // Unmapped where the library agrees; a library with no say in it is
  // retired instead, and stays mapped.
  console.log();
  for (const library of reg.libraries()) {
    const key = String(library.key);
    try {
      reg.unload(key);
      console.log(`unloaded ${key}`);
    } catch (e) {
      if (!(e instanceof GuatiaoError)) throw e;
      reg.retire(key);
      console.log(`retired ${key}, still mapped: unload said ${e.name}`);
    }
  }
}

process.exitCode = await main();
