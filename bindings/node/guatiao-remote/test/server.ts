// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * guatiao-remote-server on loopback over WebSocket, serving
 * `derived_greeter` for the greeter kinds, for the live suite in Node and
 * in Chrome alike: started once in Node, its URL provided to both. A
 * second one serves `wss://` at server TLS, with a certificate it made
 * itself, for the page alone (the test browser takes it; Node's WebSocket
 * would not). Three more serve WebTransport (`https://`), for the page alone
 * too (Node has no WebTransport): at server TLS, at none and at mutual TLS,
 * each presenting a certificate it made, which the page pins by its hash.
 *
 * The server is `GUATIAO_REMOTE_SERVER`, else the checkout's debug build;
 * the library is the one beside it. Missing, the run fails naming the
 * build that makes them.
 */

import { type ChildProcess, spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import type { TestProject } from 'vitest/node';

/** The key the server holds and the suite's links prove. */
export const REMOTE_KEY = 'remote-browser-suite';

declare module 'vitest' {
  export interface ProvidedContext {
    remoteUrl: string;
    remoteKey: string;
    serverTlsUrl: string;
    /** WebTransport servers, each with the hashes a page pins. */
    wtServerTls: Served;
    wtNone: Served;
    wtMutual: Served;
  }
}

const repo = fileURLToPath(new URL('../../../../', import.meta.url));

function library(dir: string): string {
  if (process.platform === 'win32') return path.join(dir, 'derived_greeter.dll');
  if (process.platform === 'darwin') return path.join(dir, 'libderived_greeter.dylib');
  return path.join(dir, 'libderived_greeter.so');
}

const children: ChildProcess[] = [];
let keyDir: string | undefined;

/** A server's URL and the certificate hashes it says (WebTransport's alone). */
export interface Served {
  url: string;
  hashes: string[];
}

/**
 * Starts the server with `args` and answers the URL it says it serves, and
 * for WebTransport the certificate hashes it says next.
 */
async function serve(server: string, args: string[]): Promise<Served> {
  const proc = spawn(server, args, { stdio: ['ignore', 'ignore', 'pipe'] });
  children.push(proc);
  const said: string[] = [];
  return new Promise<Served>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`the server said no address in time: ${said.join('\n')}`)), 30_000);
    let buffered = '';
    let url: string | undefined;
    proc.stderr!.setEncoding('utf8');
    proc.stderr!.on('data', (chunk: string) => {
      buffered += chunk;
      const lines = buffered.split('\n');
      buffered = lines.pop()!;
      for (const line of lines) {
        said.push(line);
        const m = /serving ((?:wss?|https):\/\/[^\s/]+)/.exec(line);
        if (m !== null) url = `${m[1]}/`;
        const h = /certificate hashes (.+)$/.exec(line.trim());
        if (url !== undefined && (h !== null || !url.startsWith('https'))) {
          clearTimeout(timer);
          resolve({ url, hashes: h === null ? [] : h[1]!.split(' ') });
        }
      }
    });
    proc.on('exit', (code) => {
      clearTimeout(timer);
      reject(new Error(`the server exited (${code}): ${said.join('\n')}`));
    });
  });
}

export async function setup(project: TestProject): Promise<void> {
  const named = process.env['GUATIAO_REMOTE_SERVER'];
  const exe = process.platform === 'win32' ? '.exe' : '';
  const server =
    named !== undefined && named !== '' ? named : path.join(repo, 'target', 'debug', `guatiao-remote-server${exe}`);
  const greeter = library(path.dirname(server));
  if (!existsSync(server) || !existsSync(greeter)) {
    throw new Error(
      `no ${server} beside ${greeter}: build them with ` +
        '`cargo build -p guatiao-source-remote -p derived_greeter --features guatiao-source-remote/ws,guatiao-source-remote/tls,guatiao-source-remote/webtransport` ' +
        '(or name the server in GUATIAO_REMOTE_SERVER)',
    );
  }
  keyDir = mkdtempSync(path.join(tmpdir(), 'guatiao-remote-'));
  const keyFile = path.join(keyDir, 'link.key');
  writeFileSync(keyFile, REMOTE_KEY);
  const serving = [
    '--kinds',
    path.join(repo, 'examples', 'greeter_kind', 'include', 'greeter_kind.kinds.json'),
    '--load',
    greeter,
  ];
  const start = async (args: string[]): Promise<string> => (await serve(server, args)).url;
  const url = await start([
    '--listen',
    'ws://127.0.0.1:0',
    '--security',
    'none',
    '--range',
    '127.0.0.1',
    '--key-file',
    keyFile,
    ...serving,
  ]);
  const serverTlsUrl = await start([
    '--listen',
    'wss://127.0.0.1:0',
    '--security',
    'server-tls',
    '--identity',
    path.join(keyDir, 'server.pem'),
    '--key-file',
    keyFile,
    ...serving,
  ]);
  project.provide('remoteUrl', url);
  project.provide('remoteKey', REMOTE_KEY);
  project.provide('serverTlsUrl', serverTlsUrl);
  const at = ['--listen', 'https://127.0.0.1:0'];
  project.provide(
    'wtServerTls',
    await serve(server, [...at, '--security', 'server-tls', '--key-file', keyFile, ...serving]),
  );
  project.provide(
    'wtNone',
    await serve(server, [...at, '--security', 'none', '--range', '127.0.0.1', '--key-file', keyFile, ...serving]),
  );
  project.provide(
    'wtMutual',
    // A client pinned, rotating certificates the page takes: Chrome then
    // refuses the link for want of a client certificate alone.
    await serve(server, [...at, '--security', 'mutual-tls', '--pin', `sha256:${'cd'.repeat(32)}`, ...serving]),
  );
}

export function teardown(): void {
  for (const child of children.splice(0)) child.kill();
  if (keyDir !== undefined) rmSync(keyDir, { recursive: true, force: true });
  keyDir = undefined;
}
