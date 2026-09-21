// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Reading a tree from an `isolateGroupBound` callback that a native thread
/// calls, which is how an engine hands a host a value.
///
/// The callback is started on a fresh OS thread through the platform's own
/// thread API, so it runs with no isolate of its own, as it would when an
/// engine calls in. Calling it through its pointer from the Dart thread
/// would run it in this isolate instead, and prove less.
library;

import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:guatiao/guatiao.dart';
// The raw type, as a consumer's own symbol-file bindings would alias it.
import 'package:guatiao/src/bindings.g.dart' show guatiao_value;
import 'package:test/test.dart';

import 'support.dart';

/// What the thread is handed, and where it writes what it read.
final class _Job extends Struct {
  @Int64()
  external int address;

  /// Distinct keys read, or -1 when the read threw.
  @Int64()
  external int result;
}

void _read(Pointer<_Job> job) {
  final tree =
      Ref.borrowed(Pointer<guatiao_value>.fromAddress(job.ref.address));
  job.ref.result = (tree.toDart()! as Map).keys.toSet().length;
}

// A Windows thread procedure: `DWORD (LPVOID)`.
int _windowsProc(Pointer<Void> job) {
  _read(job.cast());
  return 0;
}

// A POSIX start routine, `void *(void *)`. A group-bound callback cannot
// return a pointer, having no constant to fall back on when it throws, so
// this returns the pointer-sized integer the ABI puts in the same register.
int _posixProc(Pointer<Void> job) {
  _read(job.cast());
  return 0;
}

/// Runs [job] to completion on a new native thread.
void _onNativeThread(Pointer<_Job> job) {
  if (Platform.isWindows) {
    final kernel = DynamicLibrary.open('kernel32.dll');
    final create = kernel.lookupFunction<
        IntPtr Function(Pointer, IntPtr, Pointer, Pointer, Uint32, Pointer),
        int Function(
            Pointer, int, Pointer, Pointer, int, Pointer)>('CreateThread');
    final wait = kernel.lookupFunction<Uint32 Function(IntPtr, Uint32),
        int Function(int, int)>('WaitForSingleObject');
    final close =
        kernel.lookupFunction<Int32 Function(IntPtr), int Function(int)>(
            'CloseHandle');
    final proc =
        NativeCallable<Uint32 Function(Pointer<Void>)>.isolateGroupBound(
      _windowsProc,
      exceptionalReturn: 1,
    );
    try {
      final thread = create(nullptr, 0, proc.nativeFunction, job, 0, nullptr);
      expect(thread, isNot(0), reason: 'CreateThread failed');
      wait(thread, 0xFFFFFFFF);
      close(thread);
    } finally {
      proc.close();
    }
  } else {
    final libc = DynamicLibrary.process();
    final create = libc.lookupFunction<
        Int32 Function(Pointer<IntPtr>, Pointer, Pointer, Pointer),
        int Function(
            Pointer<IntPtr>, Pointer, Pointer, Pointer)>('pthread_create');
    final join = libc.lookupFunction<Int32 Function(IntPtr, Pointer),
        int Function(int, Pointer)>('pthread_join');
    final proc =
        NativeCallable<IntPtr Function(Pointer<Void>)>.isolateGroupBound(
      _posixProc,
      exceptionalReturn: 0,
    );
    final handle = calloc<IntPtr>();
    try {
      expect(create(handle, nullptr, proc.nativeFunction, job), 0,
          reason: 'pthread_create failed');
      expect(join(handle.value, nullptr), 0, reason: 'pthread_join failed');
    } finally {
      calloc.free(handle);
      proc.close();
    }
  }
}

void main() {
  setUpAll(libraryDir);

  test('a borrowed tree reads correctly on a native thread, group-bound', () {
    final value = Value.fromDart({'a': 1, 'bb': 'two', 'ccc': true});
    final job = calloc<_Job>();
    try {
      job.ref
        ..address = value.pointer.address
        ..result = -1;
      _onNativeThread(job);
      // 3 is right. 1 is a stride read as zero; -1 is the read throwing.
      expect(job.ref.result, 3);
    } finally {
      calloc.free(job);
      value.close();
    }
  });
}
