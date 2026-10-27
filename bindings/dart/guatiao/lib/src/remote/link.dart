// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A link to a peer speaking guatiao's remote protocol, in plain Dart: a
// browser page or any Dart program calls the providers a server serves.
// Everything runs on the isolate's event loop: a call the peer makes
// while one of ours is open runs as its frame arrives, and a call made
// from inside it names it as its parent through a Zone value.

import 'dart:async';
import 'dart:collection';
import 'dart:typed_data';

import 'package:clock/clock.dart';

import '../status.dart';
import 'args.dart';
import 'codec.dart';
import 'described.dart';
import 'proof.dart' as proof;
import 'socket.dart';
import 'webtransport.dart';

/// How far a link trusts its peer: the level both sides must name.
enum RemoteTrust {
  /// The same machine; the operating system's permissions are the gate.
  local(0),

  /// A network the owner isolates: a pre-shared key proved both ways. No
  /// encryption, and no defence against anyone on that network.
  trusted(1),

  /// Any network, mutual TLS: both sides proven by certificate.
  secure(2),

  /// Any network, server TLS: the server proven by its certificate, the
  /// client by the key (when the server asks for it) or a token.
  serverTls(3);

  const RemoteTrust(this.code);

  /// The byte a HELLO carries.
  final int code;
}

/// A link's security, the same choice on every transport and client.
/// WebTransport (`https://`) runs any of the three: QUIC always encrypts,
/// and the browser always checks the server's certificate.
enum RemoteSecurity {
  /// No TLS (`ws://`): the pre-shared key proved both ways, peers admitted
  /// by the server's access control list. No encryption, and no defence
  /// against anyone on that network.
  none,

  /// TLS that proves the server (`wss://`): this side proves itself by the
  /// key, when the server asks for it, or a token.
  serverTls,

  /// TLS that proves both sides (`wss://`): in a page, with a client
  /// certificate the user installed in the browser; on the Dart VM, with
  /// the identity [RemoteTls] names.
  mutualTls,
}

/// How the Dart VM's TLS knows the server, and what it presents: the
/// fingerprints (`sha256:<hex>`) of the servers it trusts, and its own
/// identity, a PEM file's text holding one certificate and its PKCS#8 key.
/// A page's WebSocket has none of this: its browser trusts the web's
/// certificate authorities and presents what the user installed. A page's
/// WebTransport (`https://`) takes [pins] alone, as the browser's
/// `serverCertificateHashes`: what guatiao-remote-server says as its
/// `certificate hashes`.
final class RemoteTls {
  const RemoteTls({this.pins = const [], this.identity});

  /// The servers trusted, by fingerprint; empty trusts the platform's
  /// certificate authorities.
  final List<String> pins;

  /// This side's certificate and key, PEM, for mutual TLS.
  final String? identity;
}

/// A message transport: one frame's body per message. A WebSocket is
/// one; another transport plugs in through [RemoteLink.over].
abstract interface class RemoteChannel {
  /// Each message's bytes, in order; done when the channel closes.
  Stream<Uint8List> get messages;

  /// Sends one message.
  void send(Uint8List body);

  /// Closes the channel; [messages] is then done.
  Future<void> close();

  /// Datagrams beside the messages, on a channel that has them
  /// (WebTransport); `null` elsewhere.
  RemoteDatagrams? get datagrams;
}

/// Datagrams beside a link's calls: each delivered whole or not at all,
/// in any order, for what a page accepts losing (a sensor's readings, a
/// wire channel's samples). A datagram is the payload alone, nothing
/// added, so a server reads it with its own stack (`Link::datagrams()` in
/// Rust); for values the payload is a wire channel's frame.
abstract interface class RemoteDatagrams {
  /// The largest payload the path takes now.
  int get maxSize;

  /// Sends one: [BadValue] past [maxSize], [Gone] once the link ended; one
  /// lost on the way is not said.
  void send(Uint8List payload);

  /// Each datagram as it arrives, until the link ends. Listened to once.
  Stream<Uint8List> get received;
}

/// An object this side serves: [slots] maps each slot's name to what
/// runs when the peer calls it, given the arguments as the slot's
/// description types them (a lent map or buffer is the one the answer
/// writes back, to change in place) and answering what the slot answers.
final class Served {
  Served(
    this.kind,
    Map<String, FutureOr<Object?> Function(List<Object?> args)> slots,
  ) : slots = Map.unmodifiable(slots);

  /// The object kind it is, by name.
  final String kind;

  final Map<String, FutureOr<Object?> Function(List<Object?> args)> slots;

  @override
  String toString() => 'Served($kind)';
}

/// What a peer serves, as its catalog lists it.
final class RemoteCatalog {
  RemoteCatalog._(this.kinds, this.libraries);

  /// Every kind a table or an object is of, by name.
  final Map<String, RemoteKind> kinds;

  /// Every library the peer serves.
  final List<RemoteLibrary> libraries;

  /// Every provider, of every library.
  Iterable<RemoteProvider> get providers =>
      libraries.expand((l) => l.providers);
}

/// A library a peer serves. Its providers answer only once
/// [RemoteLink.open] has opened it.
final class RemoteLibrary {
  RemoteLibrary._(this.id, this.version, this.stamp, this.providers);

  final String id;
  final String version;

  /// Changes whenever the library does.
  final String stamp;

  final List<RemoteProvider> providers;

  @override
  String toString() => 'RemoteLibrary($id $version)';
}

/// A provider a peer serves: a table per kind.
final class RemoteProvider {
  RemoteProvider._(this.id, this.name, this.meta, this.tables);

  final String id;
  final String name;

  /// What else it declares, or an empty map.
  final Map<String, Object?> meta;

  /// Its tables, by kind name.
  final Map<String, RemoteTable> tables;

  /// Its table of [kind], or `null` when it serves none.
  RemoteTable? table(String kind) => tables[kind];

  @override
  String toString() => 'RemoteProvider($id)';
}

/// Something calls go to: a provider's table or a peer's object.
abstract base class RemoteCallee {
  RemoteCallee._(this._link, this.kind);

  final RemoteLink _link;

  /// What its slots are.
  final RemoteKind kind;

  Target get _target;

  /// Whether it fills [slot].
  bool has(String slot);

  /// Calls [slot] with [args], typed by the kind's description: a
  /// [Served] or a [RemoteObject] where it takes an object, a modifiable
  /// map, list or `Uint8List` where it writes one back. Answers what the
  /// slot answers: plain Dart for a tree, a [RemoteObject] for an object
  /// of the peer's, the [Served] itself for one of this side's.
  Future<Object?> call(String slot, [List<Object?> args = const []]) =>
      Future.sync(() {
        final s = kind.slot(slot);
        if (!has(slot)) {
          throw Unsupported(
            Status.unsupported.code,
            'the far `${kind.name}` table does not fill `$slot`',
          );
        }
        return _link._call(_target, s, args);
      });
}

/// A provider's table of one kind.
final class RemoteTable extends RemoteCallee {
  RemoteTable._(super.link, super.kind, this.id, this.slots) : super._();

  /// The number a CALL names it by.
  final int id;

  /// The slots the far table fills; a caller has no default for the
  /// others.
  final Set<String> slots;

  @override
  Target get _target => Target.table(id);

  @override
  bool has(String slot) => slots.contains(slot);
}

/// A handle on an object the peer holds. [release] lets it go; a handle
/// lent for one call answers `Gone` once that call returns.
final class RemoteObject extends RemoteCallee {
  RemoteObject._(super.link, super.kind, this.id, {required bool lent})
    : _lent = lent,
      super._();

  /// Its id, in the peer's numbering.
  final int id;
  final bool _lent;
  bool _gone = false;

  @override
  Target get _target => Target.object(id);

  @override
  bool has(String slot) => !_gone;

  /// Whether it can still be called.
  bool get alive => !_gone && _link.alive;

  @override
  Future<Object?> call(String slot, [List<Object?> args = const []]) {
    if (_gone) {
      return Future.error(Gone(Status.gone.code, 'the object has been let go'));
    }
    return super.call(slot, args);
  }

  /// Lets the object go; the peer drops it. Idempotent. A lent handle is
  /// the lender's to end, and releasing it only ends this handle.
  void release() {
    if (_gone) return;
    _gone = true;
    _link._finalizer.detach(this);
    if (!_lent) _link._send(Release(id));
  }

  @override
  String toString() => 'RemoteObject(${kind.name} $id)';
}

/// An object of this side's, held for the peer.
final class _Export {
  _Export(this.served, this.kind, {required this.lent});
  final Served served;
  final RemoteKind kind;
  final bool lent;
}

final class _Pending {
  _Pending(this.slot);
  final RemoteSlot? slot;
  final completer = Completer<Reply>();
  Timer? timer;
}

/// The inbound call a call is made during: its parent.
final class _Parent {
  const _Parent(this.link, this.call);
  final RemoteLink link;
  final int call;
}

final Object _parentKey = Object();

/// Refuses, before anything is sent, a link that has nothing to prove
/// itself with at [trust].
void _checkProof(RemoteTrust trust, List<int>? key, String? token) {
  final hasKey = key != null && key.isNotEmpty;
  if (trust == RemoteTrust.trusted && !hasKey) {
    throw ArgumentError.value(
      key,
      'key',
      'the trusted level needs a pre-shared key',
    );
  }
  if (trust == RemoteTrust.serverTls && !hasKey && token == null) {
    throw ArgumentError(
      'at server TLS the client proves itself by a key or a token: name one',
    );
  }
}

/// A link to a peer speaking guatiao's remote protocol.
final class RemoteLink {
  RemoteLink._(
    this._channel,
    this.trust,
    this._timeout,
    this._maxFrame,
    this._maxCalls,
    this._sendMax,
    this._theirCalls,
  );

  final RemoteChannel _channel;

  /// The level the link runs at.
  final RemoteTrust trust;

  final Duration? _timeout;
  final int _maxFrame;
  final int _maxCalls;
  final int _sendMax;
  final int _theirCalls;

  final _done = Completer<String>();
  String? _why;

  int _nextCall = 1;
  int _nextExport = 1;
  final _pending = <int, _Pending>{};

  /// Calls given up on, whose late answer may carry an object to let go;
  /// the oldest forgotten past [_maxCancelled].
  final _cancelled = <int, RemoteSlot>{};
  static const int _maxCancelled = 4096;
  final _running = <int, bool>{};
  int _runningTop = 0;
  int _inFlight = 0;

  /// Calls waiting for a place under the peer's `max_calls`, each with
  /// the timer that gives it up.
  final _permits = Queue<(Completer<void>, Timer?)>();
  final _exports = <int, _Export>{};
  final _kinds = <String, RemoteKind>{};

  late final Finalizer<int> _finalizer = Finalizer<int>((id) {
    if (alive) _send(Release(id));
  });

  /// Connects to a server's WebSocket or WebTransport URL at [security],
  /// mutual TLS unless told: `ws://host:port/path` runs
  /// [RemoteSecurity.none] alone, `wss://` the two TLS modes, and
  /// `https://host:port/path` (WebTransport, a page's only) any of the
  /// three, the server's certificate pinned by [tls]'s pins or trusted as
  /// the browser trusts any. With none, [key] is proved both ways; with
  /// server TLS this side proves [key] when the server asks for it, or
  /// presents [token]; with mutual TLS a [token] goes with the
  /// certificate when the server requires one. [tls] (the Dart VM only)
  /// names the servers pinned and this side's identity. [timeout] bounds
  /// each call (`null` waits as long as the link lives),
  /// [handshakeTimeout] the handshake; [maxFrame] is the largest message
  /// this side reads, [maxCalls] how many of the peer's calls it runs at
  /// once.
  static Future<RemoteLink> connect(
    Uri url, {
    RemoteSecurity security = RemoteSecurity.mutualTls,
    List<int>? key,
    String? token,
    RemoteTls? tls,
    Duration? timeout = const Duration(seconds: 30),
    Duration handshakeTimeout = const Duration(seconds: 10),
    int maxFrame = 64 << 20,
    int maxCalls = 64,
  }) async {
    final trust = switch (security) {
      RemoteSecurity.none => RemoteTrust.trusted,
      RemoteSecurity.serverTls => RemoteTrust.serverTls,
      RemoteSecurity.mutualTls => RemoteTrust.secure,
    };
    if (url.scheme == 'https') {
      _checkProof(trust, key, token);
      final channel = await openWebTransport(
        url,
        handshakeTimeout,
        tls,
        maxFrame,
      );
      return over(
        channel,
        trust: trust,
        key: key,
        token: token,
        timeout: timeout,
        handshakeTimeout: handshakeTimeout,
        maxFrame: maxFrame,
        maxCalls: maxCalls,
      );
    }
    final encrypted = switch (url.scheme) {
      'ws' => false,
      'wss' => true,
      _ => throw ArgumentError.value(
        url,
        'url',
        'a remote URL is ws://, wss:// or https:// (WebTransport)',
      ),
    };
    if (encrypted != (security != RemoteSecurity.none)) {
      throw ArgumentError.value(
        url,
        'url',
        encrypted
            ? 'wss:// runs TLS: pass security: RemoteSecurity.serverTls or mutualTls'
            : 'ws:// carries no TLS, and the link is ${security.name}: pass '
                  'security: RemoteSecurity.none, or connect to wss://',
      );
    }
    _checkProof(trust, key, token);
    final channel = await openWebSocket(url, handshakeTimeout, tls);
    return over(
      channel,
      trust: trust,
      key: key,
      token: token,
      timeout: timeout,
      handshakeTimeout: handshakeTimeout,
      maxFrame: maxFrame,
      maxCalls: maxCalls,
    );
  }

  /// Runs the handshake over [channel] at [trust] and answers the link.
  /// The trusted level needs [key]; server TLS needs [key] or [token] (the
  /// key proved when the server's WELCOME asks for it), and the channel's
  /// own TLS must have proved the server; a secure server may ask for a
  /// [token].
  static Future<RemoteLink> over(
    RemoteChannel channel, {
    RemoteTrust trust = RemoteTrust.trusted,
    List<int>? key,
    String? token,
    Duration? timeout = const Duration(seconds: 30),
    Duration handshakeTimeout = const Duration(seconds: 10),
    int maxFrame = 64 << 20,
    int maxCalls = 64,
  }) async {
    try {
      _checkProof(trust, key, token);
    } catch (_) {
      await channel.close();
      rethrow;
    }
    final messages = StreamIterator(channel.messages);
    final mine = proof.nonce();
    channel.send(
      Hello(trust.code, mine, {
        'max_frame': maxFrame,
        'max_calls': maxCalls,
        'token': ?token,
      }).encode(),
    );
    Future<Uint8List> next() async {
      final more = await messages.moveNext().timeout(
        handshakeTimeout,
        onTimeout: () => throw Gone(
          Status.gone.code,
          'the server did not answer the handshake in time',
        ),
      );
      if (!more) {
        throw Gone(Status.gone.code, 'the server closed the link');
      }
      final body = messages.current;
      if (body.length > handshakeMaxFrame) {
        throw MalformedFrame(
          FrameFault.tooLarge,
          'a handshake frame of ${body.length} bytes',
        );
      }
      return body;
    }

    try {
      final answer = Frame.decode(await next());
      if (answer is Goodbye) {
        throw statusException(answer.status, answer.message);
      }
      if (answer is! Welcome) {
        throw MalformedFrame(FrameFault.field, 'the server sent no WELCOME');
      }
      // At server TLS a WELCOME with a proof asks for the key; one
      // without asks none.
      final asked =
          trust == RemoteTrust.trusted ||
          (trust == RemoteTrust.serverTls && answer.proof.isNotEmpty);
      if (asked) {
        if (key == null || key.isEmpty) {
          throw BadValue(
            Status.badValue.code,
            'the server proves clients by a pre-shared key, and this client has none',
          );
        }
        if (!proof.proves(
          key,
          proof.serverLabel,
          mine,
          answer.nonce,
          answer.proof,
        )) {
          throw BadValue(
            Status.badValue.code,
            "the server does not hold this link's key",
          );
        }
        channel.send(
          Proof(
            proof.proofOf(key, proof.clientLabel, mine, answer.nonce),
          ).encode(),
        );
      }
      final offered = answer.options;
      int number(String name, int fallback) {
        final v = offered is Map ? offered[name] : null;
        return v is int && v > 0 ? v : fallback;
      }

      final link = RemoteLink._(
        channel,
        trust,
        timeout,
        maxFrame,
        maxCalls,
        number('max_frame', maxFrame),
        number('max_calls', maxCalls),
      );
      link._start(messages);
      return link;
    } catch (_) {
      await messages.cancel();
      await channel.close();
      rethrow;
    }
  }

  void _start(StreamIterator<Uint8List> messages) {
    () async {
      try {
        while (alive && await messages.moveNext()) {
          _route(messages.current);
        }
        _end('the peer closed the link');
      } catch (e) {
        _end('the link failed: $e');
      }
      await messages.cancel();
    }();
  }

  /// Whether the link still runs.
  bool get alive => _why == null;

  /// Why the link ended, once it has.
  String? get why => _why;

  /// Completes with why the link ended.
  Future<String> get done => _done.future;

  /// How many objects of this side's the peer holds.
  int get objectsHeld => _exports.length;

  /// Datagrams beside the calls, over WebTransport; `null` over a
  /// WebSocket.
  RemoteDatagrams? get datagrams => _channel.datagrams;

  /// Ends the link: GOODBYE, then the channel closes. Every call waiting
  /// answers `Gone`.
  Future<void> close() async {
    if (!alive) return;
    _send(const Goodbye(0, 'the host closed the link'));
    _end('this side closed the link');
    await _channel.close();
  }

  void _send(Frame frame) => _sendBody(frame.encode());

  void _sendBody(Uint8List body) {
    if (!alive) return;
    try {
      _channel.send(body);
    } catch (e) {
      _end('a write failed: $e');
    }
  }

  void _end(String why) {
    if (_why != null) return;
    _why = why;
    for (final p in _pending.values) {
      p.timer?.cancel();
      if (!p.completer.isCompleted) {
        p.completer.completeError(Gone(Status.gone.code, why));
      }
    }
    _pending.clear();
    _exports.clear();
    for (final (waiting, timer) in _permits) {
      timer?.cancel();
      waiting.completeError(Gone(Status.gone.code, why));
    }
    _permits.clear();
    _done.complete(why);
    unawaited(_channel.close().catchError((_) {}));
  }

  void _goodbye(Status status, String why) {
    _send(Goodbye(status.code, why));
    _end(why);
  }

  /// The peer's catalog. Its kinds type every call after it.
  Future<RemoteCatalog> list() async {
    final reply = await _request((id) => ListCatalog(id), null);
    final r = FrameReader(reply.body);
    final value = r.value();
    r.end();
    return _catalogOf(value);
  }

  /// Opens [library] on the peer: its providers answer until [closeLibrary]
  /// or the link's end.
  Future<void> open(RemoteLibrary library) async {
    await _request((id) => Open(id, library.id, library.stamp), null);
  }

  /// Lets an opened library go.
  void closeLibrary(RemoteLibrary library) =>
      _send(Close(library.id, library.stamp));

  /// One round trip.
  Future<Duration> ping() async {
    final watch = Stopwatch()..start();
    await _request((id) => Ping(id), null);
    return watch.elapsed;
  }

  RemoteCatalog _catalogOf(Object? value) {
    BadValue bad(String why) =>
        BadValue(Status.badValue.code, 'the catalog: $why');
    if (value is! Map) throw bad('not a map');
    final kinds = <String, RemoteKind>{};
    for (final k in value['kinds'] as List? ?? const []) {
      final kind = RemoteKind.fromDescription(k);
      kinds[kind.name] = kind;
    }
    _kinds
      ..clear()
      ..addAll(kinds);
    final libraries = <RemoteLibrary>[];
    for (final l in value['libraries'] as List? ?? const []) {
      if (l is! Map) throw bad('a library is not a map');
      final providers = <RemoteProvider>[];
      for (final p in l['providers'] as List? ?? const []) {
        if (p is! Map) throw bad('a provider is not a map');
        final tables = <String, RemoteTable>{};
        for (final t in p['tables'] as List? ?? const []) {
          if (t is! Map) throw bad('a table is not a map');
          final kind = kinds[t['kind']];
          final id = t['table'];
          if (kind == null || id is! int) {
            throw bad('a table names no kind the catalog describes');
          }
          tables[kind.name] = RemoteTable._(
            this,
            kind,
            id,
            Set.unmodifiable((t['slots'] as List? ?? const []).cast<String>()),
          );
        }
        final meta = p['meta'];
        providers.add(
          RemoteProvider._(
            '${p['id']}',
            '${p['name'] ?? p['id']}',
            meta is Map ? Map<String, Object?>.unmodifiable(meta) : const {},
            Map.unmodifiable(tables),
          ),
        );
      }
      libraries.add(
        RemoteLibrary._(
          '${l['id']}',
          '${l['version'] ?? ''}',
          '${l['stamp'] ?? ''}',
          List.unmodifiable(providers),
        ),
      );
    }
    return RemoteCatalog._(
      Map.unmodifiable(kinds),
      List.unmodifiable(libraries),
    );
  }

  /// The inbound call this one is made during, or 0. A zone outlives the
  /// call it was made for (a timer a handler started runs in it), so the
  /// call must still be running.
  int _parent() {
    final p = Zone.current[_parentKey];
    return p is _Parent &&
            identical(p.link, this) &&
            _running.containsKey(p.call)
        ? p.call
        : 0;
  }

  /// A place under the peer's `max_calls`, waiting at most [within]
  /// (`null`: as long as the link lives), else [Busy].
  Future<void> _permit(Duration? within) {
    if (_inFlight < _theirCalls) {
      _inFlight++;
      return Future.value();
    }
    final waiting = Completer<void>();
    late final (Completer<void>, Timer?) entry;
    final timer = within == null
        ? null
        : Timer(within, () {
            if (_permits.remove(entry)) {
              waiting.completeError(
                Busy(
                  Status.busy.code,
                  "the peer's limit of calls in flight was not freed in time",
                ),
              );
            }
          });
    entry = (waiting, timer);
    _permits.add(entry);
    return waiting.future;
  }

  void _unpermit() {
    if (_permits.isNotEmpty) {
      final (waiting, timer) = _permits.removeFirst();
      timer?.cancel();
      waiting.complete();
    } else {
      _inFlight--;
    }
  }

  /// Sends the frame [build] makes for a new call id and answers its
  /// REPLY, refusing a failed one with its status. [unsent] runs when the
  /// frame never left.
  Future<Reply> _request(
    Frame Function(int id) build,
    RemoteSlot? slot, {
    void Function()? unsent,
    Duration? within,
  }) async {
    final Uint8List body;
    final int id;
    try {
      if (!alive) throw Gone(Status.gone.code, _why!);
      id = _nextCall++;
      body = build(id).encode();
      if (body.length > _sendMax) {
        throw BadValue(
          Status.badValue.code,
          'a frame of ${body.length} bytes is over the $_sendMax the peer reads',
        );
      }
    } catch (_) {
      unsent?.call();
      rethrow;
    }
    final pending = _Pending(slot);
    _pending[id] = pending;
    final timeout = within ?? _timeout;
    if (timeout != null) {
      pending.timer = Timer(timeout, () {
        if (_pending.remove(id) == null) return;
        if (slot != null) {
          if (_cancelled.length >= _maxCancelled) {
            _cancelled.remove(_cancelled.keys.first);
          }
          _cancelled[id] = slot;
        }
        _send(Cancel(id));
        pending.completer.completeError(
          Gone(Status.gone.code, 'no answer within the call\'s timeout'),
        );
      });
    }
    try {
      _channel.send(body);
    } catch (e) {
      _pending.remove(id);
      pending.timer?.cancel();
      unsent?.call();
      _end('a write failed: $e');
      throw Gone(Status.gone.code, _why!);
    }
    final reply = await pending.completer.future;
    if (reply.status != 0) {
      String words;
      try {
        words = reply.words();
      } on MalformedFrame {
        words = 'the peer refused';
      }
      throw statusException(reply.status, words);
    }
    return reply;
  }

  RemoteKind _kind(String? name) {
    final kind = name == null ? null : _kinds[name];
    if (kind == null) {
      throw NotFound(
        Status.notFound.code,
        'no kind `$name` in the catalog: list() first',
      );
    }
    return kind;
  }

  int _export(Served served, RemoteKind kind, {required bool lent}) {
    if (served.kind != kind.name) {
      throw WrongKind(
        Status.wrongKind.code,
        'a `${served.kind}` where a `${kind.name}` is taken',
      );
    }
    final id = _nextExport++;
    _exports[id] = _Export(served, kind, lent: lent);
    return id;
  }

  Future<Object?> _call(
    Target target,
    RemoteSlot slot,
    List<Object?> args,
  ) async {
    if (!alive) throw Gone(Status.gone.code, _why!);
    if (args.length != slot.args.length) {
      throw ArgumentError(
        '${args.length} arguments for `${slot.name}`, which takes '
        '${slot.args.length}',
      );
    }
    final parent = _parent();
    // One bound for the whole call: waiting for a place, then the answer,
    // timed by the zone's clock as its timers are.
    final started = clock.stopwatch()..start();
    if (parent == 0) await _permit(_timeout);
    final timeout = _timeout;
    final left = timeout == null
        ? null
        : timeout - started.elapsed < Duration.zero
        ? Duration.zero
        : timeout - started.elapsed;
    final lent = <int>[];
    final handed = <int>[];
    final givenBack = <RemoteObject>[];
    try {
      final wire = List<Object?>.of(args);
      try {
        _objectsOut(slot, wire, lent, handed, givenBack);
      } catch (_) {
        handed.forEach(_exports.remove);
        rethrow;
      }
      var sent = true;
      final Reply reply;
      try {
        reply = await _request(
          (id) => Call(id, parent, target, slot.index, encodeArgs(slot, wire)),
          slot,
          unsent: () {
            sent = false;
            handed.forEach(_exports.remove);
          },
          within: left,
        );
      } finally {
        // Handed back to its owner, who keeps it: nothing to release.
        if (sent) {
          for (final v in givenBack) {
            v._gone = true;
            _finalizer.detach(v);
          }
        }
      }
      final decoded = decodeAnswer(slot, reply.body);
      var w = 0;
      for (var i = 0; i < slot.args.length; i++) {
        final a = slot.args[i];
        final tree = {'value', 'map', 'list'}.contains(a.type);
        if (tree && a.pass == 'lent') {
          _writeBack(args[i], decoded.writebacks[w++]);
        } else if (a.type == 'bytes_mut') {
          final into = args[i]! as Uint8List;
          final from = decoded.writebacks[w++]! as Uint8List;
          into.setRange(0, from.length.clamp(0, into.length), from);
        }
      }
      return _answerOf(slot, decoded.answer);
    } finally {
      for (final id in lent) {
        _exports.remove(id);
      }
      if (parent == 0) _unpermit();
    }
  }

  /// Puts each object argument in [wire] as it crosses: a [Served]
  /// exported (its id in [lent] or [handed]), a handle on the peer's object
  /// as the peer's own (in [givenBack] when taken).
  void _objectsOut(
    RemoteSlot slot,
    List<Object?> wire,
    List<int> lent,
    List<int> handed,
    List<RemoteObject> givenBack,
  ) {
    for (var i = 0; i < wire.length; i++) {
      final a = slot.args[i];
      if (a.type != 'object') continue;
      final v = wire[i];
      if (v is Served) {
        final id = _export(v, _kind(a.kind), lent: a.pass == 'lent');
        (a.pass == 'lent' ? lent : handed).add(id);
        wire[i] = ObjectRef(id);
      } else if (v is RemoteObject && identical(v._link, this)) {
        if (v._gone) {
          throw Gone(Status.gone.code, 'the object has been let go');
        }
        wire[i] = ObjectRef(v.id, receivers: true);
        if (a.pass == 'taken') givenBack.add(v);
      }
    }
  }

  void _writeBack(Object? into, Object? from) {
    if (into is Map && from is Map) {
      into
        ..clear()
        ..addAll(from);
    } else if (into is List && from is List) {
      into
        ..clear()
        ..addAll(from);
    }
  }

  Object? _answerOf(RemoteSlot slot, Object? answer) {
    if (answer is! ObjectRef) return answer;
    if (answer.receivers) {
      return _takeOwn(answer.id).served;
    }
    return _handle(_kind(slot.answer?.kind), answer.id, lent: false);
  }

  /// An object of this side's the peer hands back as itself. One lent
  /// for a call stays lent: it cannot be handed over.
  _Export _takeOwn(int id) {
    final own = _exports[id];
    if (own == null) {
      throw Gone(Status.gone.code, 'the object has been let go');
    }
    if (own.lent) {
      throw WrongKind(
        Status.wrongKind.code,
        'an object lent for a call cannot be handed over',
      );
    }
    return _exports.remove(id)!;
  }

  /// Lets go one object the peer handed over in a call that will not take
  /// it: the peer's is released, this side's own given back dropped.
  void _letGo(ObjectRef ref) {
    if (!ref.receivers) {
      _send(Release(ref.id));
    } else if (_exports[ref.id]?.lent == false) {
      _exports.remove(ref.id);
    }
  }

  /// Lets go what a call of the peer's handed over when it is refused
  /// before running; a lent object ends with the call on the peer's side.
  /// A call on an object this side does not hold, or a slot past its
  /// kind's, cannot be read: what it handed stays until the link ends.
  void _letGoHanded(Call c) {
    final export = c.target.object ? _exports[c.target.id] : null;
    if (export == null || c.slot >= export.kind.slots.length) return;
    final slot = export.kind.slots[c.slot];
    final List<Object?> args;
    try {
      args = decodeArgs(slot, c.args);
    } on MalformedFrame {
      return;
    }
    for (var i = 0; i < args.length; i++) {
      final ref = args[i];
      if (ref is ObjectRef && slot.args[i].pass != 'lent') _letGo(ref);
    }
  }

  RemoteObject _handle(RemoteKind kind, int id, {required bool lent}) {
    final handle = RemoteObject._(this, kind, id, lent: lent);
    if (!lent) _finalizer.attach(handle, id, detach: handle);
    return handle;
  }

  void _route(Uint8List body) {
    if (!alive) return;
    if (body.length > _maxFrame) {
      _goodbye(
        Status.badValue,
        'a frame of ${body.length} bytes is over the $_maxFrame allowed',
      );
      return;
    }
    final Frame frame;
    try {
      frame = Frame.decode(body);
    } on MalformedFrame catch (e) {
      _goodbye(
        Status.badValue,
        'the peer sent a malformed frame: ${e.message}',
      );
      return;
    }
    switch (frame) {
      case Reply(:final call):
        final pending = _pending.remove(call);
        if (pending != null) {
          pending.timer?.cancel();
          pending.completer.complete(frame);
        } else {
          final slot = _cancelled.remove(call);
          if (slot != null) _settleCancelled(slot, frame);
        }
      case Pong(:final n):
        final pending = _pending.remove(n);
        if (pending != null) {
          pending.timer?.cancel();
          pending.completer.complete(Reply(n, 0, Uint8List(0)));
        }
      case Call():
        _inbound(frame);
      case ListCatalog(:final call) || Open(:final call):
        _send(
          Reply(
            call,
            Status.unsupported.code,
            (FrameWriter()..text('this side serves no catalog')).done(),
          ),
        );
      case Close():
        break;
      case Cancel(:final call):
        if (_running.containsKey(call)) _running[call] = true;
      case Release(:final object):
        _exports.remove(object);
      case Ping(:final n):
        _send(Pong(n));
      case Goodbye(:final message):
        _end('the peer ended the link: $message');
      case Hello() || Welcome() || Proof():
        _goodbye(Status.badValue, 'a handshake frame after the handshake');
    }
  }

  /// A cancelled call's late answer: an object in it is let go, the
  /// peer's released and one of this side's dropped.
  void _settleCancelled(RemoteSlot slot, Reply reply) {
    if (reply.status != 0 || slot.answer?.type != 'object') return;
    try {
      final answer = decodeAnswer(slot, reply.body).answer;
      if (answer is ObjectRef) _letGo(answer);
    } on MalformedFrame {
      // Nobody waits for it.
    }
  }

  void _refuse(int call, Status status, String why) =>
      _send(Reply(call, status.code, (FrameWriter()..text(why)).done()));

  void _inbound(Call c) {
    if (c.parent != 0 && !_pending.containsKey(c.parent)) {
      _letGoHanded(c);
      _refuse(c.call, Status.gone, 'the call it was made during has returned');
      return;
    }
    final top = c.parent == 0;
    if (top && _runningTop >= _maxCalls) {
      _letGoHanded(c);
      _refuse(
        c.call,
        Status.busy,
        'the peer is running as many calls as it allows',
      );
      return;
    }
    if (top) _runningTop++;
    _running[c.call] = false;
    unawaited(
      _serve(c).whenComplete(() {
        _running.remove(c.call);
        if (top) _runningTop--;
      }),
    );
  }

  Future<void> _serve(Call c) async {
    final target = c.target;
    final export = target.object ? _exports[target.id] : null;
    if (export == null) {
      _refuse(
        c.call,
        target.object ? Status.gone : Status.notFound,
        target.object
            ? 'the object has been let go'
            : 'no such table on this link: this side serves no providers',
      );
      return;
    }
    if (c.slot >= export.kind.slots.length) {
      _refuse(c.call, Status.notFound, 'the kind has no such slot');
      return;
    }
    final slot = export.kind.slots[c.slot];
    final handler = export.served.slots[slot.name];
    if (handler == null) {
      _letGoHanded(c);
      _refuse(
        c.call,
        Status.unsupported,
        'the `${export.kind.name}` this side serves does not fill `${slot.name}`',
      );
      return;
    }
    final List<Object?> args;
    try {
      args = decodeArgs(slot, c.args);
    } on MalformedFrame catch (e) {
      _refuse(c.call, Status.badValue, e.message);
      return;
    }
    // Every object handed over is taken before a refusal is answered, so
    // none is left held for a call that never ran.
    final lentHandles = <RemoteObject>[];
    final takenHandles = <RemoteObject>[];
    GuatiaoException? refused;
    for (var i = 0; i < args.length; i++) {
      final ref = args[i];
      if (ref is! ObjectRef) continue;
      final lent = slot.args[i].pass == 'lent';
      if (refused != null) {
        if (!lent) _letGo(ref);
        continue;
      }
      try {
        if (ref.receivers) {
          final own = lent ? _exports[ref.id] : _takeOwn(ref.id);
          if (own == null) {
            throw Gone(Status.gone.code, 'the object has been let go');
          }
          args[i] = own.served;
        } else {
          final handle = _handle(_kind(slot.args[i].kind), ref.id, lent: lent);
          (lent ? lentHandles : takenHandles).add(handle);
          args[i] = handle;
        }
      } on GuatiaoException catch (e) {
        refused = e;
        if (!lent && !ref.receivers) _send(Release(ref.id));
      }
    }
    if (refused != null) {
      for (final h in lentHandles) {
        h._gone = true;
      }
      for (final h in takenHandles) {
        h.release();
      }
      _refuse(
        c.call,
        Status.fromCode(refused.code) ?? Status.internalError,
        refused.message,
      );
      return;
    }
    Object? answer;
    try {
      answer = await runZoned(
        () => Future.sync(() => handler(args)),
        zoneValues: {_parentKey: _Parent(this, c.call)},
      );
    } on GuatiaoException catch (e) {
      if (_running[c.call] != true) {
        _send(Reply(c.call, e.code, (FrameWriter()..text(e.message)).done()));
      }
      return;
    } catch (e) {
      if (_running[c.call] != true) {
        _refuse(c.call, Status.internalError, '$e');
      }
      return;
    } finally {
      for (final h in lentHandles) {
        h._gone = true;
      }
    }
    if (_running[c.call] == true || !alive) return;
    int? exported;
    try {
      Object? wire = answer;
      final givenBack = answer is RemoteObject && identical(answer._link, this)
          ? answer
          : null;
      if (answer is Served) {
        exported = _export(answer, _kind(slot.answer?.kind), lent: false);
        wire = ObjectRef(exported);
      } else if (givenBack != null) {
        wire = ObjectRef(givenBack.id, receivers: true);
      }
      final reply = Reply(c.call, 0, encodeAnswer(slot, wire, args)).encode();
      if (reply.length > _sendMax) {
        throw BadValue(
          Status.badValue.code,
          'an answer of ${reply.length} bytes is over the $_sendMax the peer '
          'reads',
        );
      }
      if (givenBack != null) {
        givenBack._gone = true;
        _finalizer.detach(givenBack);
      }
      _sendBody(reply);
    } catch (e) {
      if (exported != null) _exports.remove(exported);
      final (status, words) = switch (e) {
        GuatiaoException() => (
          Status.fromCode(e.code) ?? Status.internalError,
          e.message,
        ),
        ArgumentError() => (Status.badValue, '${e.message}'),
        _ => (Status.internalError, '$e'),
      };
      _refuse(c.call, status, words);
    }
  }
}
