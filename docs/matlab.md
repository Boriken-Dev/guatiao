# MATLAB and Octave: host and provider

MATLAB and GNU Octave play both parts. As a **host** they load libraries
written in any language and call their providers; as a **provider** they
write a library, classes overriding a kind's methods, that a host in the
same session calls, whatever language the host is written in. Both use
the `+guatiao` package: `classdef` and functions in the subset MATLAB
(R2024a and later) and Octave (8.4 and later) both run, over one small
MEX file built from C on the user's machine, which opens the guatiao
library at run time.

```matlab
pkg install guatiao-VERSION.tar.gz    % Octave, from a release: builds the MEX file
pkg load guatiao
guatiao.build()                             % MATLAB, after installing guatiao.mltbx
```

Building needs a C compiler that `mex` or `mkoctfile` supports and libffi
(`libffi-dev` on Debian and Ubuntu, `libffi-devel` on Fedora; macOS and
Octave's Windows installer carry their own; MATLAB with Visual C++ takes
vcpkg's `libffi:x64-windows-static-md` through `guatiao.build('FfiDir',
...)`). The package loads the guatiao library on its first native call:
the one `guatiao.use_library(path)` named, else `GUATIAO_LIBRARY` (a file
or a folder), else the platform's library search. The library must be
built for the process's architecture; on Windows ARM64, MATLAB and Octave
are x64 builds and need an x64 build of the library.

## A kind, declared once

A kind is the table its C header declares. Here it is a class in the kind
author's package, inheriting `guatiao.Kind`, that describes the slots once
(their order, what each takes and answers, the floor hash the header
defines) and has a method per slot. A kind with a description (the
`<crate>.kinds.json` a kind crate commits beside its header, or one
another language wrote) is written, not declared:

```matlab
guatiao.write_kinds('../include/greeter_kind.kinds.json', '+greeterkind')
```

writes a class per kind into the package folder, the description's docs
as its help; `'Check', true` writes nothing and raises `guatiao:Stale`
when a class is not what it would write. The greeter kinds below are its
output:

```matlab
classdef Greeter < guatiao.Kind
  properties (Constant, Hidden)
    GreeterKind = guatiao.Kind.describe('greeter', 2032737757, ...
      guatiao.slot('greet', {'name', 'string'}, 'Returns', 'map', 'Required', true, 'Fallible', true), ...
      guatiao.slot('shout', {'name', 'string'}, 'Returns', 'string'), ...
      guatiao.slot('start', {'listener', guatiao.taken('greeterkind.Listener')}, ...
                   'Returns', 'greeterkind.Conversation', 'Fallible', true), ...
      guatiao.slot('greet_into', {'name', 'string', 'listener', guatiao.lent('greeterkind.Listener')}, 'Fallible', true), ...
      guatiao.slot('greet_from', {'config', guatiao.taken('map')}, 'Returns', 'map', 'Fallible', true), ...
      guatiao.slot('check', {'config', guatiao.lent('map')}, 'Returns', 'bool', 'Fallible', true))
  end
  methods
    function out = greet(self, name), out = self.call('greet', name); end
    function out = shout(self, name), out = self.call('shout', name); end
    % ... one method per slot
  end
end
```

A map, list or value is borrowed by default; `guatiao.lent` makes it the
caller's tree for the call, changed in place; `guatiao.taken` hands it
over, as Rust's arguments by value do. An object kind (`'Object', true`)
is passed lent or taken, and its `destroy` slot is `close()`. The
description is a constant named after the class (`GreeterKind`), so a
class inheriting two kinds inherits two names. The example kinds are
`examples/greeter_kind/matlab/+greeterkind`.

## As a host

```matlab
reg = guatiao.Registry('my-host', '1.0');
reg.watch_dir('/opt/plugins', 'kind=greeter');
reg.add_source_file('/opt/app/libguatiao_source_py.so', struct('dirs', {{'/opt/app/py'}}));
reg.refresh();

g = reg.bind('py_greeter_hello', 'greeterkind.Greeter');   % an instance of the kind's class
answer = g.greet('ana');            % a guatiao.Value: {greeting: "hello, ana"}
greeting = answer.at('greeting');
greeting.data()
answer.close();

chat = g.start(my_listener);        % an object of a class implementing Listener
chat.say('hi');
chat.close();                       % destroy: the greeter lets the listener go
reg.close();
```

A registry is a handle object with methods (`scan_dir`, `providers`,
`bind`, `create`, `retire`, `unload`, `refresh`); every answer that is data
is plain MATLAB data. A remote is an instance of the kind's class whose
methods call the table; it checks its registry or instance on every call,
and a refusal raises an error whose identifier is
`guatiao:<Status>:provider`, with the provider's words.

## As a provider

```matlab
classdef Hello < greeterkind.Greeter & greeterkind.Counter
  properties (Constant, Hidden)
    Provider = guatiao.provider('DisplayName', 'Hello, in M')
  end
  properties
    asked = 0
  end
  methods
    function out = greet(self, name)
      if isempty(name), error('guatiao:BadValue', 'nobody to greet'); end
      out = struct('greeting', ['hello, ' name]);
    end
    function n = count(self)
      self.asked = self.asked + 1; n = self.asked;
    end
  end
end

classdef Shouter < greeterkind.Greeter
  properties (Constant, Hidden)
    Provider = guatiao.provider('Config', guatiao.Config( ...
      'prefix', guatiao.field('string', 'Doc', 'What every greeting starts with.', 'minLength', 1)))
  end
  properties
    prefix = ''
  end
  methods
    function self = Shouter(config)
      if nargin, self.prefix = config.prefix; end
    end
    function out = greet(self, name)
      out = struct('greeting', [self.prefix ' ' upper(name)]);
    end
  end
end
```

A provider is a handle class inheriting the kinds it serves and
overriding their methods; a required slot it leaves alone is refused when
the library is made. A configuration is a `guatiao.Config` field list:
the host checks a configuration against its schema before anything is
built, and the class's constructor receives it as a struct with the
defaults filled in. The folder is the library:
`guatiao.library_of('mlgreeter')` takes its id, version, authors,
description and licence from its `DESCRIPTION` (Octave's package format;
MATLAB's `Contents.m` version line works too), its licence files as
notices, and its providers from the classes in its namespace that carry a
`Provider` constant. `reg.register_library(lib)` registers it, and any
host in this session, Rust, C, Python, Go, Julia, R or M, calls its
providers through the same tables it calls any library's through.

## Threads

M code runs only on the interpreter's thread. A slot an M provider or
object serves runs there **while a call into guatiao is in progress**: a
host's own call (a remote's method, a registry call), a library calling
back during one, or code run by `guatiao.serving`, which C code of one's
own that calls tables directly (a MEX file, `calllib`) needs. **Called from
any other thread, or with no such call in progress, it does not run**: it
answers `Unsupported` at once, with "M code runs only on the
interpreter's thread, inside a call into guatiao", touching no
interpreter object; a provider's `available` answers false with that
reason; a destroy asked for there is queued and done on the interpreter's
thread at the package's next call. Nothing waits, so nothing deadlocks; a
library that calls providers from worker threads of its own needs
providers written in another language.

The method runs under `mexCallMATLABWithTrap`, so an M error comes back
as a value and never unwinds through the C or Rust frames between the
library and the interpreter; it crosses as a status (`Internal` with its
message, or the status its identifier names). Every error the package
raises is raised at the top of the MEX file's own call, never inside a
library's.

## What MATLAB and Octave do not do that Python does

- **Run a provider on a library's own thread.** Python takes the GIL on
  any thread; the interpreter here belongs to one thread, and is reached
  only while a call into guatiao is in progress.
- **Serve another process.** guatiao-source-py serves Python modules to any host;
  nothing serves M classes to a host that is not running MATLAB or Octave,
  which would need one embedded in a bridge.
- **Run without a compiler.** The MEX file is built on the user's machine
  (or by `pkg install`), against libffi.
- **Run a kind's default body** for a slot a provider leaves out, as a
  Rust caller does: a remote raises `Unsupported`, as in Python, Node, Go,
  Julia and R.
- **Index a package function's result in place, in Octave.**
  `guatiao.intake.at(v, 'a').data()` works in MATLAB only; name the result
  first.

## Where MATLAB and Octave differ

- MATLAB holds text as UTF-16 and Octave as UTF-8; the MEX file converts,
  so both see the same characters and a value holds UTF-8.
- Octave's `mexCallMATLABWithTrap` drops an error's message; the package
  catches errors in M first, so a provider's words cross in both.
- Octave 11 warns when an object of a class with two handle superclasses
  (a provider serving two kinds) is deleted, unless the class defines
  `delete` itself; the example provider does.
- Octave builds the MEX file at `pkg install`; MATLAB builds it with
  `guatiao.build()` after installing the toolbox. MATLAB on Windows needs a
  Visual C++ it supports (R2024a: Visual Studio 2022).

## The examples

- `examples/matlab_host/host.m` scans for greeters in any language,
  registers the M library, optionally adds the guatiao-source-py bridge, and
  calls every slot of the kind; its README has the commands.
- `examples/matlab_provider/mlgreeter/` is the library above.
- `examples/greeter_kind/matlab/+greeterkind/` holds the greeter kinds,
  written from their description.
- `examples/tally_kind/matlab/+tallykind/` holds the tally kinds, which
  exist only as a description, written from it; the suite serves them
  from M and calls every slot.
