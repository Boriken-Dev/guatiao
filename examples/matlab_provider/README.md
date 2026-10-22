# Greeters written in M

[`mlgreeter`](mlgreeter/) is a library of greeters written in M for
MATLAB and GNU Octave: the same kinds the Rust, C and Python greeters
serve
([`../greeter_kind/matlab/+greeterkind`](../greeter_kind/matlab/+greeterkind/Greeter.m)),
offered to any host in the same session.

- `+mlgreeter/Hello.m` serves the greeter and counter kinds with one
  instance of its own; its `start` hands back a `Chat`
  (`+mlgreeter/Chat.m`), a conversation holding the listener it was handed.
- `+mlgreeter/Shouter.m` is built from a configuration, its schema read
  from the `guatiao.Config` in its `Provider` declaration.

The folder is the library: `guatiao.library_of('mlgreeter')` takes its id,
version and meta from [`mlgreeter/DESCRIPTION`](mlgreeter/DESCRIPTION)
(Octave's package format), its licence from `COPYING`, and its providers
from the classes in `+mlgreeter` that carry a `Provider` constant.

```matlab
addpath bindings/matlab/inst examples/greeter_kind/matlab examples/matlab_provider/mlgreeter
reg = guatiao.Registry('my-host', '1.0');
reg.register_library(guatiao.library_of('mlgreeter'));
g = reg.bind('mlgreeter_hello', 'greeterkind.Greeter');
answer = g.greet('ana');
answer.data()      % struct('greeting', 'hello, ana')
```

[`../matlab_host`](../matlab_host/) registers it beside greeters written
in every other language.
