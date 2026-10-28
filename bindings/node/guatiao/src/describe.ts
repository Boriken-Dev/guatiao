// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A kind written as a TypeScript interface, read into the description
 * `#[guatiao::kind]` writes for a Rust trait.
 *
 * `describeInterfaces(files)` reads every exported interface of the files
 * through the compiler (the `typescript` package, 7.0 or newer) and answers
 * a document `kindsOf` loads and `guatiao-kind` and the Dart generator write
 * code from. The types are the ones of `guatiao/kinds` (`u32`, `i64`,
 * `Lent<MapRef>`, `Taken<Listener>`, ...); each is found by its declaration,
 * so an alias or a renamed import of one reads the same.
 *
 * @module
 */

import path from 'node:path';

import { SIZE, Status, Tag, VALUE } from './abi.js';
import { BadValue, raiseForStatus } from './errors.js';
import type { KindDescription, KindsDocument, TypeDescription } from './kinds.js';
import { methodName, slotName } from './kinds.js';
import * as n from './native.js';

type Api = typeof import('typescript/unstable/sync');
type Ast = typeof import('typescript/unstable/ast');
type Node = import('typescript/unstable/ast').Node;
type TypeNode = import('typescript/unstable/ast').TypeNode;
type SourceFile = import('typescript/unstable/ast').SourceFile;
type InterfaceDeclaration = import('typescript/unstable/ast').InterfaceDeclaration;
type MethodSignature = import('typescript/unstable/ast').MethodSignatureDeclaration;
type Checker = import('typescript/unstable/sync').Checker;

export interface DescribeOptions {
  /**
   * The `tsconfig.json` the files are read through, which says where
   * `guatiao/kinds` and the files' other imports are. Default: the nearest
   * one above each file, else an inferred project.
   */
  readonly project?: string;
}

/** Why the interfaces are not a description: the file and place, and what is wrong. */
export class DescribeError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'DescribeError';
  }
}

async function compiler(): Promise<{ api: Api; ast: Ast }> {
  try {
    const [api, ast] = await Promise.all([import('typescript/unstable/sync'), import('typescript/unstable/ast')]);
    return { api, ast };
  } catch (cause) {
    const e = cause as { code?: string };
    if (e.code !== 'ERR_MODULE_NOT_FOUND' && e.code !== 'ERR_PACKAGE_PATH_NOT_EXPORTED') throw cause;
    throw new DescribeError(
      'describing TypeScript interfaces reads them with the compiler: install the `typescript` package, version 7.0 or newer (npm install --save-dev typescript)',
      { cause },
    );
  }
}

const SCALARS = new Set(['i8', 'i16', 'i32', 'i64', 'isize', 'u8', 'u16', 'u32', 'u64', 'usize', 'f32', 'f64']);
const RESERVED = new Set(
  (
    'break case catch class const continue debugger default delete do else enum export extends false finally for ' +
    'function if import in instanceof new null return super switch this throw true try typeof var void while with ' +
    'yield let static implements interface package private protected public await arguments eval'
  ).split(' '),
);

/** A type the description names, or that the slot has no answer. */
type Spelled = TypeDescription | 'void';

class Reader {
  readonly #kinds = new Map<string, { decl: InterfaceDeclaration; name: string }>();
  readonly #order: string[] = [];
  readonly #queue: InterfaceDeclaration[] = [];

  constructor(
    readonly ast: Ast,
    readonly api: Api,
    readonly checker: Checker,
  ) {}

  // ---- where a node is, for a refusal ------------------------------------------------------------

  at(node: Node): string {
    const file = node.getSourceFile();
    const { line, character } = file.getLineAndCharacterOfPosition(node.getStart(file));
    return `${file.fileName}:${line + 1}:${character + 1}`;
  }

  refuse(node: Node, why: string): never {
    throw new DescribeError(`${this.at(node)}: ${why}`);
  }

  // ---- comments and tags --------------------------------------------------------------------------

  /** A comment's text: the plain text of each part, a link as written. */
  #text(comment: unknown): string | undefined {
    if (comment === undefined) return undefined;
    if (typeof comment === 'string') return comment;
    const { SyntaxKind } = this.ast;
    return (comment as readonly Node[])
      .map((part) => (part.kind === SyntaxKind.JSDocText ? (part as unknown as { text: string }).text : part.getText()))
      .join('');
  }

  /** The doc text above a declaration, as `///` lines read in Rust: one leading space and trailing spaces dropped. */
  doc(node: Node): string | undefined {
    const blocks = (node.jsDoc ?? []) as readonly { comment?: unknown }[];
    const text = this.#text(blocks.at(-1)?.comment);
    if (text === undefined) return undefined;
    const lines = text.split('\n').map((line) => line.trimEnd());
    const joined = lines.join('\n').replace(/^\n+|\n+$/g, '');
    return joined === '' ? undefined : joined;
  }

  tag(node: Node, name: string): string | undefined {
    for (const block of (node.jsDoc ?? []) as readonly { tags?: readonly { tagName: { text: string }; comment?: unknown }[] }[]) {
      for (const tag of block.tags ?? []) {
        if (tag.tagName.text === name) return (this.#text(tag.comment) ?? '').trim();
      }
    }
    return undefined;
  }

  // ---- a name's declarations -----------------------------------------------------------------------

  declarations(name: Node): Node[] {
    const { SyntaxKind } = this.ast;
    const id = name.kind === SyntaxKind.QualifiedName ? (name as unknown as { right: Node }).right : name;
    let symbol = this.checker.getSymbolAtLocation(id);
    if (symbol === undefined) return [];
    if (symbol.flags & this.api.SymbolFlags.Alias) symbol = this.checker.getAliasedSymbol(symbol);
    return symbol.declarations.flatMap((handle) => {
      const node = handle.resolve();
      return node === undefined ? [] : [node];
    });
  }

  // ---- a type, spelled as a description spells it ------------------------------------------------------

  /** `node` as an argument or an answer. */
  type(node: TypeNode, position: 'arg' | 'answer', where: string): Spelled {
    const { SyntaxKind } = this.ast;
    switch (node.kind) {
      case SyntaxKind.ParenthesizedType:
        return this.type((node as unknown as { type: TypeNode }).type, position, where);
      case SyntaxKind.StringKeyword:
        return { type: position === 'arg' ? 'str' : 'text' };
      case SyntaxKind.BooleanKeyword:
        return { type: 'bool' };
      case SyntaxKind.VoidKeyword:
        if (position === 'answer') return 'void';
        return this.refuse(node, `${where}: void is not an argument`);
      case SyntaxKind.NumberKeyword:
      case SyntaxKind.BigIntKeyword:
        return this.refuse(
          node,
          `${where}: ${node.getText()} says nothing of the C type; write u8..u32, i8..i32, f32, f64, or i64, u64, isize, usize from guatiao/kinds`,
        );
      case SyntaxKind.UnionType: {
        const types = (node as unknown as { types: readonly TypeNode[] }).types;
        const nullish = (t: TypeNode): boolean =>
          t.kind === SyntaxKind.UndefinedKeyword ||
          (t.kind === SyntaxKind.LiteralType && (t as unknown as { literal: Node }).literal.kind === SyntaxKind.NullKeyword);
        const rest = types.filter((t) => !nullish(t));
        if (rest.length !== 1 || rest.length === types.length) {
          return this.refuse(node, `${where}: a union is not a type a slot has, except \`Ref | null\` for a value that may be null`);
        }
        const inner = this.type(rest[0]!, position, where);
        if (position !== 'arg' || inner === 'void' || inner.type !== 'value' || inner.pass !== 'borrowed') {
          return this.refuse(node, `${where}: only a borrowed value (\`Ref | null\`) may be null`);
        }
        return { ...inner, nullable: true };
      }
      case SyntaxKind.TypeReference:
        return this.reference(node, position, where);
      default:
        return this.refuse(node, `${where}: ${node.getText()} is not a type a slot has`);
    }
  }

  reference(node: TypeNode, position: 'arg' | 'answer', where: string): Spelled {
    const ref = node as unknown as { typeName: Node; typeArguments?: readonly TypeNode[] };
    const found = this.declarations(ref.typeName);
    if (found.length === 0) {
      return this.refuse(node, `${where}: ${ref.typeName.getText()} is not found; is the import there, and does the project resolve guatiao/kinds?`);
    }
    const marked = found.find((d) => this.tag(d, 'guatiaoType') !== undefined);
    const tag = marked === undefined ? undefined : this.tag(marked, 'guatiaoType');
    const { SyntaxKind } = this.ast;
    if (tag !== undefined) return this.marker(tag, node, ref.typeArguments ?? [], position, where);
    const decl = found[0]!;
    if (decl.getSourceFile().isDeclarationFile && ref.typeName.getText() === 'Uint8Array') {
      if (position === 'arg') return { type: 'bytes' };
      return this.refuse(node, `${where}: a slot answers text, a value, a map, a list or an object, not bytes`);
    }
    if (decl.kind === SyntaxKind.TypeAliasDeclaration) {
      const alias = decl as unknown as { typeParameters?: readonly Node[]; type: TypeNode };
      if (alias.typeParameters?.length) {
        return this.refuse(node, `${where}: ${ref.typeName.getText()} takes type parameters, which a slot's type cannot`);
      }
      return this.type(alias.type, position, where);
    }
    if (decl.kind === SyntaxKind.InterfaceDeclaration) {
      const iface = decl as InterfaceDeclaration;
      if (this.isObjectKind(iface) && position === 'answer') return { type: 'object', kind: this.kindNamed(iface) };
      return this.refuse(
        node,
        this.isObjectKind(iface)
          ? `${where}: an object is passed as Taken<${iface.name.text}> or Lent<${iface.name.text}>`
          : `${where}: ${iface.name.text} is not a type a slot has; an interface is one only as an object kind (extends ObjectKind)`,
      );
    }
    return this.refuse(node, `${where}: ${ref.typeName.getText()} is not a type a slot has`);
  }

  /** The type a `@guatiaoType` declaration stands for. */
  marker(tag: string, node: Node, args: readonly TypeNode[], position: 'arg' | 'answer', where: string): Spelled {
    if (SCALARS.has(tag)) return { type: tag };
    switch (tag) {
      case 'bytes_mut':
        if (position === 'arg') return { type: 'bytes_mut' };
        return this.refuse(node, `${where}: bytes_mut is an argument`);
      case 'tree_value':
      case 'tree_map':
      case 'tree_list':
        return this.tree(tag, node, 'borrowed', position, where);
      case 'borrowed':
      case 'taken':
      case 'lent': {
        if (args.length !== 1) return this.refuse(node, `${where}: ${tag} wraps one type`);
        if (position !== 'arg') return this.refuse(node, `${where}: ${tag} is how an argument crosses; an answer is built for the caller`);
        const target = this.target(args[0]!, where);
        if (target.object !== undefined) {
          if (tag === 'borrowed') return this.refuse(node, `${where}: an object is passed taken or lent`);
          return { type: 'object', kind: this.kindNamed(target.object), pass: tag };
        }
        return this.tree(target.tree!, node, tag, position, where);
      }
      case 'answer_value':
      case 'answer_map':
      case 'answer_list':
        if (position === 'answer') return { type: tag.slice('answer_'.length) };
        return this.refuse(node, `${where}: ${node.getText()} is what a slot answers`);
      case 'object_kind':
        return this.refuse(node, `${where}: ObjectKind is what an interface extends, not a type`);
      default:
        return this.refuse(node, `${where}: ${tag} is not a type this version of guatiao reads`);
    }
  }

  tree(tag: string, node: Node, pass: 'borrowed' | 'taken' | 'lent', position: 'arg' | 'answer', where: string): TypeDescription {
    const type = tag.slice('tree_'.length);
    if (position === 'answer') return this.refuse(node, `${where}: a slot answers MapAnswer, ListAnswer or ValueAnswer`);
    if (type === 'list' && pass === 'borrowed') return this.refuse(node, `${where}: a list is passed Taken or Lent`);
    return { type, pass };
  }

  /** What `Taken<X>` or `Lent<X>` wraps: a tree class or an object kind. */
  target(node: TypeNode, where: string): { tree?: string; object?: InterfaceDeclaration } {
    const { SyntaxKind } = this.ast;
    if (node.kind === SyntaxKind.ParenthesizedType) return this.target((node as unknown as { type: TypeNode }).type, where);
    if (node.kind === SyntaxKind.TypeReference) {
      const found = this.declarations((node as unknown as { typeName: Node }).typeName);
      const decl = found[0];
      if (decl !== undefined) {
        const tag = found.map((d) => this.tag(d, 'guatiaoType')).find((t) => t !== undefined);
        if (tag === 'tree_value' || tag === 'tree_map' || tag === 'tree_list') return { tree: tag };
        if (decl.kind === SyntaxKind.TypeAliasDeclaration) {
          return this.target((decl as unknown as { type: TypeNode }).type, where);
        }
        if (decl.kind === SyntaxKind.InterfaceDeclaration && this.isObjectKind(decl as InterfaceDeclaration)) {
          return { object: decl as InterfaceDeclaration };
        }
      }
    }
    return this.refuse(node, `${where}: ${node.getText()} is not a tree (Ref, MapRef, ListRef) or an object kind (an interface extending ObjectKind)`);
  }

  // ---- kinds ----------------------------------------------------------------------------------------

  isObjectKind(decl: InterfaceDeclaration): boolean {
    const { SyntaxKind } = this.ast;
    for (const clause of decl.heritageClauses ?? []) {
      if (clause.token !== SyntaxKind.ExtendsKeyword) continue;
      for (const base of clause.types) {
        const tag = this.declarations(base.expression).map((d) => this.tag(d, 'guatiaoType')).find((t) => t !== undefined);
        if (tag === 'object_kind') return true;
        this.refuse(base, `${decl.name.text} extends ${base.expression.getText()}: a kind extends only ObjectKind`);
      }
    }
    return false;
  }

  /** The name a kind has in a description, queuing its interface to be described. */
  kindNamed(decl: InterfaceDeclaration): string {
    const key = `${decl.getSourceFile().fileName}:${decl.pos}`;
    const known = this.#kinds.get(key);
    if (known !== undefined) return known.name;
    const given = this.tag(decl, 'kind');
    const name = given !== undefined && given !== '' ? given : slotName(decl.name.text);
    for (const other of this.#kinds.values()) {
      if (other.name === name) this.refuse(decl, `two interfaces are the kind ${name}; name one with @kind`);
    }
    this.#kinds.set(key, { decl, name });
    this.#order.push(key);
    this.#queue.push(decl);
    return name;
  }

  slot(member: Node, kind: string, object: boolean): KindDescription['slots'][number] {
    const { SyntaxKind } = this.ast;
    if (member.kind !== SyntaxKind.MethodSignature) {
      return this.refuse(member, `${kind}: a kind's slots are methods; ${member.getText()} is not one`);
    }
    const m = member as MethodSignature;
    if (m.name.kind !== SyntaxKind.Identifier) return this.refuse(m, `${kind}: a slot is named by an identifier`);
    const method = (m.name as unknown as { text: string }).text;
    const where = `${kind}.${method}`;
    const slot = slotName(method);
    if (methodName(slot) !== method) {
      return this.refuse(m, `${where}: its slot name ${slot} would be written ${methodName(slot)} in TypeScript; write the method as ${methodName(slot)}`);
    }
    if (object && slot === 'destroy') return this.refuse(m, `${where}: an object kind's table holds destroy before its slots`);
    if (m.typeParameters?.length) return this.refuse(m, `${where}: a slot has no type parameters`);
    if (m.type === undefined) return this.refuse(m, `${where}: say what it answers (: void for nothing)`);
    const args: TypeDescription[] = m.parameters.map((p, i) => {
      if (p.name.kind !== SyntaxKind.Identifier) return this.refuse(p, `${where}: argument ${i} is named by an identifier`);
      if (p.dotDotDotToken !== undefined || p.questionToken !== undefined || p.initializer !== undefined) {
        return this.refuse(p, `${where}: argument ${i} is a plain, required parameter`);
      }
      if (p.type === undefined) return this.refuse(p, `${where}: argument ${i} has a type`);
      const spelled = this.type(p.type, 'arg', `${where} argument ${i}`);
      if (spelled === 'void') return this.refuse(p, `${where}: argument ${i} is void`);
      const plain = (p.name as unknown as { text: string }).text;
      const unreserved = plain.endsWith('_') && RESERVED.has(plain.slice(0, -1)) ? plain.slice(0, -1) : plain;
      return { name: slotName(unreserved), ...spelled };
    });
    const answer = this.type(m.type, 'answer', `${where} answer`);
    const optional = m.postfixToken?.kind === SyntaxKind.QuestionToken;
    const docText = this.doc(m);
    return {
      name: slot,
      required: !optional,
      fallible: this.tag(m, 'throws') !== undefined,
      args,
      ...(answer === 'void' ? {} : { answer }),
      ...(docText === undefined ? {} : { doc: docText }),
    };
  }

  /** Describes `decl`, which `kindNamed` has named. */
  describe(decl: InterfaceDeclaration): KindDescription {
    const name = this.kindNamed(decl);
    const label = decl.name.text;
    if (decl.typeParameters?.length) this.refuse(decl, `${label}: a kind has no type parameters`);
    const object = this.isObjectKind(decl);
    const slots = decl.members.map((member) => this.slot(member, label, object));
    const seen = new Set<string>();
    for (const slot of slots) {
      if (seen.has(slot.name)) this.refuse(decl, `${label}: two slots are ${slot.name}`);
      seen.add(slot.name);
    }
    // The library reads the kind whether or not its hash is pinned: what no
    // kind may declare is refused there.
    let floorHash: number;
    try {
      floorHash = floorHashOf(object, slots);
    } catch (error) {
      if (!(error instanceof BadValue)) throw error;
      return this.refuse(decl, `${label}: ${error.message}`);
    }
    const pinned = this.tag(decl, 'floorHash');
    if (pinned !== undefined) {
      floorHash = Number(pinned);
      if (!/^\d+$/.test(pinned) || floorHash > 0xffffffff) this.refuse(decl, `${label}: @floorHash is a 32-bit unsigned integer, not ${pinned}`);
    }
    const docText = this.doc(decl);
    return { name, floor_hash: floorHash, object, ...(docText === undefined ? {} : { doc: docText }), slots };
  }

  /** The kinds queued so far, each described once, then those their slots reach. */
  drain(): KindDescription[] {
    const done: KindDescription[] = [];
    const byKey = new Map<string, KindDescription>();
    for (let i = 0; i < this.#queue.length; i += 1) {
      const decl = this.#queue[i]!;
      byKey.set(`${decl.getSourceFile().fileName}:${decl.pos}`, this.describe(decl));
    }
    for (const key of this.#order) done.push(byKey.get(key)!);
    return done;
  }
}

/**
 * The floor hash a kind gets when its interface pins none, computed by the
 * library (`guatiao_kind_floor_hash`): FNV-1a (32-bit) over `provider` or
 * `object` and every slot in table order, each as
 * `name(argument types)->answer` in the spelling Rust would give it, so
 * that an interface repeating a trait's slots hashes as the trait does. An
 * object is spelled by its kind's name, which Rust spells by its trait's:
 * pin `@floorHash` where the two differ. Loads the library.
 */
export function floorHashOf(object: boolean, slots: readonly KindDescription['slots'][number][]): number {
  const lib = n.core();
  const out = n.alloc(4);
  const why = n.alloc(SIZE.value);
  try {
    const status = lib.fn.guatiao_kind_floor_hash!(n.str(JSON.stringify({ object, slots })), lib.alloc, out, why);
    if (status !== Status.OK) {
      raiseForStatus(status, n.u32(why, VALUE.tag) === Tag.STRING ? n.stringText(why + BigInt(VALUE.payload)) : '');
    }
    return n.u32(out);
  } finally {
    n.freeNode(why);
    n.release(why);
    n.release(out);
  }
}

/**
 * The kinds the exported interfaces of `files` declare, and the object kinds
 * their slots reach, as the document a kind crate commits beside its header.
 * Needs the `typescript` package, 7.0 or newer; a `DescribeError` names the
 * place in a file that is not a kind, or says it is missing.
 */
export async function describeInterfaces(files: string | readonly string[], options: DescribeOptions = {}): Promise<KindsDocument> {
  const { api, ast } = await compiler();
  // The compiler names a file with forward slashes on every platform.
  const named = (file: string): string => path.resolve(file).split(path.sep).join('/');
  const list = (typeof files === 'string' ? [files] : [...files]).map(named);
  if (list.length === 0) throw new DescribeError('no file to read the interfaces of');
  const client = new api.API({ cwd: process.cwd() });
  try {
    const snapshot = client.updateSnapshot(
      options.project === undefined ? { openFiles: list } : { openProjects: [named(options.project)] },
    );
    const projectOf = (file: string): InstanceType<Api['Project']> => {
      const found =
        options.project === undefined
          ? snapshot.getDefaultProjectForFile(file)
          : snapshot.getProject(named(options.project));
      if (found === undefined) throw new DescribeError(`${file} is in no project the compiler could open`);
      return found;
    };
    const project = projectOf(list[0]!);
    const reader = new Reader(ast, api, project.checker);
    const { SyntaxKind } = ast;
    for (const file of list) {
      if (projectOf(file).id !== project.id) {
        throw new DescribeError(`${file} is in another project than ${list[0]}: name the project to read them through`);
      }
      const source: SourceFile | undefined = project.program.getSourceFile(file);
      if (source === undefined) throw new DescribeError(`${file} is not part of the project ${project.configFileName}`);
      let found = 0;
      for (const statement of source.statements) {
        if (statement.kind !== SyntaxKind.InterfaceDeclaration) continue;
        const decl = statement as InterfaceDeclaration;
        if (!(decl.modifierFlags & api.ModifierFlags.Export)) continue;
        reader.kindNamed(decl);
        found += 1;
      }
      if (found === 0) throw new DescribeError(`${file} exports no interface, so it declares no kind`);
    }
    return { kinds: reader.drain() };
  } finally {
    client.close();
  }
}
