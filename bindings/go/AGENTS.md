# guatiao (Go) — API header

Go bindings over the `guatiao` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the module, so it names no
repository path.

Module `github.com/Boriken-Dev/guatiao/bindings/go`, Go 1.26 or later,
64-bit platforms, cgo (a C compiler at build time: GCC or clang; on
Windows a MinGW-style one, such as llvm-mingw or MinGW-w64 GCC). No
dependencies outside the standard library, and no native library of its
own: guatiao is loaded at run time.

## Packages

| import | what |
| --- | --- |
| `.../bindings/go` (package `guatiao`) | `Value`, `Ref`, `MapRef`, `ListRef`, `Number`, `Tag`, `Status`, the errors, `NewValue`, `SchemaOf`, `Redact`, `UseLibrary` |
| `.../bindings/go/serde` | `Parse`, `Emit`: JSON, TOML, YAML |
| `.../bindings/go/intake` | forms, paths, the flat projection |
| `.../bindings/go/civil` | `Date`, `Time`, `DateTime`, `Span`, `Length`, instants as `time.Time` |

## Finding the native library

Nothing native is touched until the first call that needs it; the
library is then found in this order:

1. `guatiao.UseLibrary(path)`, called before that first call: a file
   exporting the ABI, whatever it is called, or a directory. Naming
   another file once one is loaded is an error; naming the same one is
   not.
2. `GUATIAO_LIBRARY`: a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib`, or a file (taken as it is for
   `guatiao`; `guatiao_serde` and `guatiao_intake` are looked for beside
   it under their own names).
3. The platform's own library search, by file name.

Not found is a `*guatiao.LibraryNotFound` (`Basename`, `Looked`). An
export the loaded build lacks is a `*guatiao.MissingSymbol` (`Path`,
`Symbol`, `Feature`) when it is used; a file missing an export every
build has is refused when it is opened. A library stays loaded for the
process.

## Values

`*Value` owns a root node; `Close()` frees it (and answers nil; a second
call does nothing). A Value dropped unclosed is freed when the garbage
collector finds it and every `Ref` into it unreachable. `Ref` addresses a
node by a path of keys and indices, walked again on every access, never a
cached address: a write beside a node can move the array it sits in.
`MapRef` and `ListRef` embed a `Ref` over a map and a list; `Value`
embeds a `Ref` onto its root. The zero `Ref` names nothing.

```go
v, err := guatiao.NewValue(map[string]any{"host": "10.0.0.1", "port": 5900, "tags": []string{"a", "b"}})
defer v.Close()
m, err := v.Map()
m.Set("port", 5901)
tags, _ := v.Key("tags").List()
tags.Push("c")
v.Key("port").IntOr(22)   // 5901
m.Keys()                  // [host port tags]
var back struct{ Host string `guatiao:"host"` }
v.Decode(&back)
```

**Building**: `NewValue(x, opts...) (*Value, error)` converts Go data (see
below); `Absent() *Value`, `Null()`, `NewMap()`, `NewList()`. Option
`InAllocator(alloc unsafe.Pointer)`: the address of a `guatiao_alloc` the
caller keeps alive while every tree built through it lives. A failed
conversion frees what it built.

**Reading** (on `Ref`, so on `Value` too): `Key(k)` and `Index(i)` extend
the path (negative counts from the end) without checking; `Tag()`,
`IsAbsent()`, `IsNull()`; `BoolOr`, `IntOr(int64)` (a signed 64-bit
integer; a fraction, an exponent, another kind or out of range falls
back, never truncates), `FloatOr`, `StringOr`; `Text() (string, bool)`,
`NumberText() (Number, bool)`, `Bytes() ([]byte, bool)` (a copy);
`Any() (any, error)` (plain data); `Decode(dst any) error`; `Map()
(MapRef, error)` and `List() (ListRef, error)` (`ErrWrongKind` for another
kind, `ErrNotFound` for nothing there); `Clone() (*Value, error)`;
`Path()`, the path in the intake grammar.

**`MapRef`**: `Len`, `Has(k)`, `Get(k) (Ref, bool)`, `Set(k, x) error`
(replaces in place), `Delete(k) (bool, error)`, `Keys()`, `All()
iter.Seq2[string, Ref]`, `Clear()`, `CopyFrom(src Ref)`. Keys compare as
UTF-8 bytes and keep insertion order.

**`ListRef`**: `Len`, `At(i) (Ref, bool)`, `Push(xs ...any) error`,
`RemoveAt(i)`, `Clear()`, `All() iter.Seq2[int, Ref]`, and `Set(i, x)` and
`Insert(i, x)`, which are O(n): the C surface only appends and removes, so
they copy every element, clear and push back.

**`Number`** is a number's exact text (`type Number string`): `Int64()`,
`Uint64()` (strict), `Float64()` (nearest; an error when not finite).

A closed Value, or a view whose source is closed or whose call has
returned, reads as absent and answers `ErrClosed` from every call that
returns an error. A borrowed view answers `ErrReadOnly` from every write.

## Go values and trees

`NewValue`, `Set`, `Push` and every kind call convert Go data; `Decode`
converts back, as `encoding/json` does:

| Go | value |
| --- | --- |
| `nil`, nil pointer or interface | null (a nil pointer *field* is left out) |
| `bool` | boolean |
| `int*`, `uint*`, `float*`, `Number`, `*big.Int` | number, its exact text (a float's shortest; `-0` kept; NaN and infinities refused) |
| `string` | string (UTF-8 or refused) |
| `[]byte` | bytes |
| slice, array | list (decoding into an array refuses another length) |
| `map[K]V`, K string, integer, bool or TextMarshaler | map, keys written sorted (integers by value) |
| struct | map: exported fields by `guatiao` tag, else `json` tag, else name; `-` skips, `omitempty` drops a zero value; untagged embedded structs lend their fields |
| `time.Time` | an instant's text, UTC (`2026-10-21T13:45:00.5Z`) |
| `time.Duration` | an exact length `[seconds, nanoseconds]`; decoding reads its three spellings; negative or past ~292 years refused |
| `Marshaler` (`MarshalGuatiao() (any, error)`), `Unmarshaler` (`UnmarshalGuatiao(Ref) error`) | what it says |
| `encoding.TextMarshaler` / `TextUnmarshaler` | string |
| `*Value`, `Ref`, `MapRef`, `ListRef` | a deep copy; decoding into `*Value` takes one |

Decoding into `any` gives `nil`, `bool`, `Number`, `string`, `[]byte`,
`[]any`, `map[string]any`. Decoding refuses a number that does not fit its
target and never rounds; an absent node leaves the target as it was; keys
a struct has no field for are ignored. Both directions stop past 128
nested containers. A failure is a `*guatiao.Error` with `BadValue` naming
the path (`at agent[1].port: want an integer, got string`).

**`SchemaOf[T]() (*Value, error)`** (and `SchemaFor(reflect.Type)`): the
JSON Schema of what `T` encodes to, properties in field order. A struct
refuses keys it does not declare; a field is required unless it is a
pointer (which may also be null), `omitempty`, or has a default; a
`schema` tag adds keywords, `schema:"minLength=1,default=hey"`, each value
read as JSON when it is JSON and as text otherwise. Integer types state
their bounds; `time.Time` is `date-time`, `time.Duration` and the civil
types their forms; a `GuatiaoSchema() map[string]any` method states a
type's own. A recursive type is refused.

## serde: JSON, TOML, YAML

```go
v, err := serde.Parse(`{"port": 5900}`, serde.JSON)
text, err := serde.Emit(v, serde.YAML, serde.Pretty())
```

`Parse(text, format, opts...) (*Value, error)`; `Emit(x, format, opts...)
(string, error)` takes a Value, a view or plain data. Options: `Pretty()`,
`How(flags)` (`BytesDataURI`, `BytesBase64`, `BytesArray`, `BytesRefuse`,
or'd with `ReadDataURIs`), `InAllocator(alloc)`. A document that does not
parse is a `*serde.ParseError` (`Format`, `Line`, `Column` from 1, the
column in characters, 0 when the parser did not say; `Reason`, which never
quotes the document); `errors.Is(err, guatiao.ErrBadValue)`. A TOML
document must be a map (`ErrWrongKind`). TOML and YAML need a
`guatiao_serde` built with them (`MissingSymbol` naming the feature
otherwise). A number keeps its exact text, except that an exponent is
read back with its sign (`1e400` is `1e+400`).

## intake

```go
why, err := intake.Check(schema, form)              // nil, or the error map
sections, err := intake.Layout(schema, form)         // []intake.Section{Section, Fields}
shown, err := intake.IsVisible(schema, form, "key", values)
ref, ok, err := intake.At(value, "agent[1].name")    // borrowed
form, err := intake.ForSchema(schema)                // a Value to close
form, err := intake.FormFor(form, schema, "path")    // a Value to close, or nil
ref, ok, err := intake.Resolve(schema, "auth.password")
keys, err := intake.FlatKeys(schema, "auth")         // ErrWrongKind for an unknown key
store, err := intake.Flatten(schema, "auth", value)  // map[string]string
value, err := intake.Unflatten(schema, "auth", store) // a Value to close, or nil
```

Every schema, form, value and values argument is a Value, a view or plain
data; `At` and `Resolve` take a Value or a view and answer a `Ref`
borrowed from it: read-only, found again on every access, `ErrClosed`
once the source is closed; `Clone()` copies it out. `Unflatten` answers
nil only for "nothing stored"; a store that is not strings to strings is
`ErrWrongKind`.

## civil: dates, times, spans, exact lengths

The forms are text in a value, fixed at
<https://boriken-dev.github.io/guatiao/encodings/>, and every text is read
and written by the library's own functions.

```go
d, err := civil.NewDate(2026, time.October, 21)      // checked
d2, err := d.Add(civil.Span{Months: 1})
t, err := civil.ParseTime("13:45:00.5")
dt, err := civil.ParseDateTime("2026-10-21T13:45:00.5")
at, err := civil.ParseTimestamp("2026-10-21T19:15:00+05:30") // time.Time, UTC
text, err := civil.FormatTimestamp(at)               // "...Z"
s, err := civil.ParseSpan("1h 30m")                  // ISO 8601 or friendly
text, err = civil.Normalize(civil.SpanForm, "1h 30m") // "PT1H30M"
l, err := civil.ReadLength(value)                    // [s, ns], seconds, {secs, nanos}
```

- **`Date`** (`Year`, `Month time.Month`, `Day`), **`Time`** (`Hour`,
  `Minute`, `Second`, `Nanosecond`), **`DateTime`** (`Date`, `Time`): a
  civil date and time to the nanosecond, no zone. `New*` and every
  `MarshalText` check through the library: `ErrBadValue` for a day the
  month lacks, a time off the clock, or a field past its C width.
  `Compare`, `String`, `MarshalText`/`UnmarshalText`, `Add(Span)` on
  `Date` and `DateTime`.
- **An instant is a `time.Time`**, nanoseconds kept; read in UTC, the
  offset it was written with not kept. Outside the years 0 to 9999 is
  `ErrBadValue`.
- **`Length`** (`Seconds uint64`, `Nanoseconds uint32`): an exact length
  to 18446744073709551615 seconds; `LengthOf(time.Duration)`,
  `Length.Duration()` (`ErrBadValue` past ~292 years); written `[s, ns]`.
- **`Span`**: eight signed fields with one sign for the whole; `==`
  compares unit by unit (`P1D` is not `PT24H`); `ParseSpan`, `Negated`,
  `IsZero`, `IsNegative`, `String`; fields of both signs or a unit out of
  range are `ErrBadValue` when written.
- **Text the grammar refuses is a `*civil.Unparsed`** (`Form`, `At` the
  byte offset in the UTF-8 text, `Expected`), an `ErrBadValue`; it never
  quotes the text.

## Secrets

```go
shown, err := guatiao.Redact(schema, value, "[redacted]")
```

Every field a schema marks (`x-sensitive` or `writeOnly`) is replaced at
any depth, and whatever the schema does not describe is hidden too. Local
`$ref`s are followed. `ErrWrongKind` for a schema that is not a map.

## Errors

A failing `guatiao_status` is a `*guatiao.Error` (`Status`, `Message`: the
other side's own words, empty when it gave none), matched with
`errors.Is` against `ErrBadValue`, `ErrAllocFailed`, `ErrWrongKind`,
`ErrNotFound`, `ErrNullArgument`, `ErrGone`, `ErrUnsupported`,
`ErrBusy`, `ErrInternal`. The `Status` constants are `OK`, `BadValue`,
`AllocFailed`, `WrongKind`, `NotFound`, `NullArgument`, `Gone`,
`Unsupported`, `Busy`, `Internal`; `guatiao.Errorf(status, format,
args...)` builds one. `ErrClosed`, `ErrReadOnly`, `*LibraryNotFound`,
`*MissingSymbol` and `*FloorMismatch` are this binding's own;
`*serde.ParseError` and `*civil.Unparsed` unwrap to `ErrBadValue`.

## Threads

A Value is not safe for concurrent use; reading one from several
goroutines at once is.
