# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Dates, times, instants, spans and exact lengths: each form is text in a
# value (or, for an exact length, [seconds, nanoseconds]), read and
# written by the library's own functions so every grammar has one
# implementation.

using Dates

"""
    Timestamp(seconds, nanoseconds)

An instant: seconds from the Unix epoch and the nanoseconds counted
forward from them, years 0000 to 9999. `Timestamp(dt::DateTime)` takes a
`DateTime` as UTC; `DateTime(ts)` answers one in UTC, dropping what is
below a millisecond toward the past.
"""
struct Timestamp
    seconds::Int64
    nanoseconds::UInt32
    function Timestamp(seconds::Integer, nanoseconds::Integer)
        0 <= nanoseconds < 1_000_000_000 || throw(GuatiaoError(BadValue, "nanoseconds below a second: $nanoseconds"))
        return new(Int64(seconds), UInt32(nanoseconds))
    end
end
const UNIX_EPOCH_MS = Dates.value(DateTime(1970, 1, 1))
function Timestamp(dt::DateTime)
    ms = Dates.value(dt) - UNIX_EPOCH_MS
    return Timestamp(fld(ms, 1000), mod(ms, 1000) * 1_000_000)
end
Dates.DateTime(ts::Timestamp) =
    DateTime(Dates.UTM(UNIX_EPOCH_MS + ts.seconds * 1000 + div(ts.nanoseconds, 1_000_000)))
Base.isless(a::Timestamp, b::Timestamp) = (a.seconds, a.nanoseconds) < (b.seconds, b.nanoseconds)

"""
    Span(; years, months, weeks, days, hours, minutes, seconds, nanoseconds)

A length in calendar units, each counted on its own, one sign for the
whole span: `Span(days = 1, hours = 2)` is `P1DT2H`. Equal unit by unit,
so `Span(days = 1) != Span(hours = 24)`. Checked by the library when
built: fields of both signs, or nanoseconds of a second or more, are a
`GuatiaoError` `BadValue`. `string(s)` is its ISO 8601 text; `-s`,
`iszero`, `isnegative`; `date + s` and `datetime + s` apply it;
`Dates.CompoundPeriod(s)` and `Span(::Dates.CompoundPeriod)` convert.
"""
struct Span
    years::Int64
    months::Int64
    weeks::Int64
    days::Int64
    hours::Int64
    minutes::Int64
    seconds::Int64
    nanoseconds::Int32
    function Span(y::Integer, mo::Integer, w::Integer, d::Integer, h::Integer, mi::Integer, s::Integer, ns::Integer)
        span = new(y, mo, w, d, h, mi, s, ns)
        format_fields(:span, GSpan(y, mo, w, d, h, mi, s, ns))   # the library's check
        return span
    end
end
Span(; years = 0, months = 0, weeks = 0, days = 0, hours = 0, minutes = 0, seconds = 0, nanoseconds = 0) =
    Span(years, months, weeks, days, hours, minutes, seconds, nanoseconds)
gspan(s::Span) = GSpan(s.years, s.months, s.weeks, s.days, s.hours, s.minutes, s.seconds, s.nanoseconds)
Span(g::GSpan) = Span(g.years, g.months, g.weeks, g.days, g.hours, g.minutes, g.seconds, g.nanoseconds)
Base.string(s::Span) = format_fields(:span, gspan(s))
Base.show(io::IO, s::Span) = print(io, "Span(\"", string(s), "\")")
Base.:-(s::Span) = Span(-s.years, -s.months, -s.weeks, -s.days, -s.hours, -s.minutes, -s.seconds, -s.nanoseconds)
Base.iszero(s::Span) = s == Span()
isnegative(s::Span) = any(<(0), (s.years, s.months, s.weeks, s.days, s.hours, s.minutes, s.seconds, s.nanoseconds))

function Span(cp::Dates.CompoundPeriod)
    f = Dict{Symbol,Int64}()
    for p in cp.periods
        unit = p isa Year ? :years : p isa Month ? :months : p isa Week ? :weeks : p isa Day ? :days :
               p isa Hour ? :hours : p isa Minute ? :minutes : p isa Second ? :seconds :
               p isa Millisecond ? :ms : p isa Microsecond ? :us : :ns
        f[unit] = get(f, unit, 0) + Dates.value(p)
    end
    ns = get(f, :ms, 0) * 1_000_000 + get(f, :us, 0) * 1_000 + get(f, :ns, 0)
    secs = get(f, :seconds, 0) + div(ns, 1_000_000_000)
    ns = rem(ns, 1_000_000_000)
    return Span(get(f, :years, 0), get(f, :months, 0), get(f, :weeks, 0), get(f, :days, 0),
        get(f, :hours, 0), get(f, :minutes, 0), secs, ns)
end
Span(p::Dates.Period) = Span(Dates.CompoundPeriod(p))
Dates.CompoundPeriod(s::Span) = Year(s.years) + Month(s.months) + Week(s.weeks) + Day(s.days) +
    Hour(s.hours) + Minute(s.minutes) + Second(s.seconds) + Nanosecond(s.nanoseconds)

"""
    Length(seconds, nanoseconds)

An exact length of time, never negative: whole seconds (to
18446744073709551615) and the nanoseconds below a second, written
`[seconds, nanoseconds]`. `Length(p::Dates.Period)` from a fixed period;
`Dates.Nanosecond(l)` (an `InexactError` past about 292 years).
"""
struct Length
    seconds::UInt64
    nanoseconds::UInt32
    function Length(seconds::Integer, nanoseconds::Integer)
        (seconds >= 0 && 0 <= nanoseconds < 1_000_000_000) ||
            throw(GuatiaoError(BadValue, "an exact length is never negative, its nanoseconds below a second"))
        return new(UInt64(seconds), UInt32(nanoseconds))
    end
end
function Length(p::Dates.FixedPeriod)
    ns = Int128(Dates.value(Nanosecond(p)))
    ns < 0 && throw(GuatiaoError(BadValue, "an exact length is never negative"))
    return Length(div(ns, 1_000_000_000), rem(ns, 1_000_000_000))
end
Dates.Nanosecond(l::Length) = Nanosecond(Int64(Int128(l.seconds) * 1_000_000_000 + l.nanoseconds))

# ---- the C functions ---------------------------------------------------------------

const FORMS = (:date, :time, :datetime, :timestamp, :span)
fields_type(form::Symbol) = form === :date ? GDate : form === :time ? GTime : form === :datetime ? GDateTime :
    form === :timestamp ? GTimestamp : form === :span ? GSpan : throw(ArgumentError("unknown form: $form"))

function take_text!(v::Ref{GValue})
    p = Base.unsafe_convert(Ptr{GValue}, v)
    try
        load_tag(p) == TAG_STRING || return ""
        return text_of(unsafe_load(payload_ptr(GString, p)))
    finally
        guatiao_value_free(p)
    end
end

function finish_text(status::UInt32, err::Ref{GTextError}, form::Symbol)
    expected = GC.@preserve err begin
        p = Ptr{GValue}(Base.unsafe_convert(Ptr{GTextError}, err) + fieldoffset(GTextError, 2))
        msg = load_tag(p) == TAG_STRING ? text_of(unsafe_load(payload_ptr(GString, p))) : ""
        guatiao_value_free(p)
        msg
    end
    status == 0 && return nothing
    status == UInt32(BadValue) && throw(Unparsed(form, Int(err[].at), isempty(expected) ? "a valid text" : expected))
    check(status, expected)
end

function parse_fields(form::Symbol, text::AbstractString)
    T = fields_type(form)
    s = String(text)
    out = Ref{T}()
    err = Ref(GTextError(0, ZERO_VALUE))
    f = form === :date ? guatiao_date_parse : form === :time ? guatiao_time_parse :
        form === :datetime ? guatiao_datetime_parse : form === :timestamp ? guatiao_timestamp_parse : guatiao_span_parse
    status = GC.@preserve s out err f(binding_alloc(), gstr(s), out, err)
    finish_text(status, err, form)
    return out[]
end

function format_fields(form::Symbol, fields)
    out = Ref(ZERO_VALUE)
    r = Ref(fields)
    f = form === :date ? guatiao_date_format : form === :time ? guatiao_time_format :
        form === :datetime ? guatiao_datetime_format : form === :timestamp ? guatiao_timestamp_format : guatiao_span_format
    status = GC.@preserve out r f(binding_alloc(), r, out)
    text = GC.@preserve out take_text!(out)
    status == UInt32(BadValue) && throw(GuatiaoError(BadValue, "the fields are not a $form"))
    check(status)
    return text
end

"""
    normalize(form::Symbol, text) -> String

`text` read as `form` (`:date`, `:time`, `:datetime`, `:timestamp`,
`:span`) and written in its canonical spelling, to the nanosecond: an
instant in UTC, a fraction without trailing zeros, a span in ISO 8601.
`Unparsed` for text the grammar refuses.
"""
function normalize(form::Symbol, text::AbstractString)
    fields_type(form)
    s = String(text)
    out = Ref(ZERO_VALUE)
    err = Ref(GTextError(0, ZERO_VALUE))
    f = form === :date ? guatiao_date_normalize : form === :time ? guatiao_time_normalize :
        form === :datetime ? guatiao_datetime_normalize : form === :timestamp ? guatiao_timestamp_normalize : guatiao_span_normalize
    status = GC.@preserve s out err f(binding_alloc(), gstr(s), out, err)
    result = GC.@preserve out take_text!(out)
    finish_text(status, err, form)
    return result
end

gdate(d::Date) = (Dates.year(d) in 0:9999 || throw(GuatiaoError(BadValue, "a date in the years 0000 to 9999"));
    GDate(Dates.year(d), Dates.month(d), Dates.day(d)))
gtime(t::Time) = GTime(Dates.hour(t), Dates.minute(t), Dates.second(t),
    Dates.millisecond(t) * 1_000_000 + Dates.microsecond(t) * 1_000 + Dates.nanosecond(t))
gtime(dt::DateTime) = GTime(Dates.hour(dt), Dates.minute(dt), Dates.second(dt), Dates.millisecond(dt) * 1_000_000)
gdatetime(dt::DateTime) = GDateTime(gdate(Date(dt)), gtime(dt))
gtimestamp(ts::Timestamp) = GTimestamp(ts.seconds, ts.nanoseconds)

julia_date(g::GDate) = Date(g.year, g.month, g.day)
julia_time(g::GTime) = Time(Nanosecond(Int64(g.hour) * 3_600_000_000_000 + Int64(g.minute) * 60_000_000_000 +
    Int64(g.second) * 1_000_000_000 + g.nanosecond))
julia_datetime(g::GDateTime) = DateTime(g.date.year, g.date.month, g.date.day, g.time.hour, g.time.minute,
    g.time.second, div(g.time.nanosecond, 1_000_000))

"""
    fromtext(T, text)

A date (`Date`), a time of day (`Time`, to the nanosecond), a civil date
and time (`DateTime`, what is below a millisecond dropped toward the
past), an instant (`Timestamp`) or a span (`Span`) read from its text by
the library's grammar (`Unparsed` when it refuses).
"""
fromtext(::Type{Date}, text::AbstractString) = julia_date(parse_fields(:date, text))
fromtext(::Type{Time}, text::AbstractString) = julia_time(parse_fields(:time, text))
fromtext(::Type{DateTime}, text::AbstractString) = julia_datetime(parse_fields(:datetime, text))
function fromtext(::Type{Timestamp}, text::AbstractString)
    g = parse_fields(:timestamp, text)
    return Timestamp(g.seconds, g.nanoseconds)
end
fromtext(::Type{Span}, text::AbstractString) = Span(parse_fields(:span, text))

"""
    totext(x) -> String

`x` (a `Date`, `Time`, `DateTime`, `Timestamp` or `Span`) in its form's
canonical text.
"""
totext(d::Date) = format_fields(:date, gdate(d))
totext(t::Time) = format_fields(:time, gtime(t))
totext(dt::DateTime) = format_fields(:datetime, gdatetime(dt))
totext(ts::Timestamp) = format_fields(:timestamp, gtimestamp(ts))
totext(s::Span) = string(s)
Base.string(ts::Timestamp) = totext(ts)
Base.show(io::IO, ts::Timestamp) = print(io, "Timestamp(\"", totext(ts), "\")")

function Base.:+(d::Date, s::Span)
    out = Ref{GDate}()
    a, b = Ref(gdate(d)), Ref(gspan(s))
    status = GC.@preserve a b out guatiao_date_add_span(a, b, out)
    check(status, "the span moves the date outside 0000-9999, or has time units")
    return julia_date(out[])
end
function Base.:+(dt::DateTime, s::Span)
    out = Ref{GDateTime}()
    a, b = Ref(gdatetime(dt)), Ref(gspan(s))
    status = GC.@preserve a b out guatiao_datetime_add_span(a, b, out)
    check(status, "the span moves the date and time outside 0000-9999")
    return julia_datetime(out[])
end

"""
    readlength(n) -> Length

An exact length read from a node in any of its three spellings
(`[seconds, nanoseconds]`, a number of seconds, `{secs, nanos}`), by the
library's own reader.
"""
function readlength(p::Ptr{GValue}, path::Vector{Key} = Key[])
    secs = Ref{UInt64}(0)
    nanos = Ref{UInt32}(0)
    status = guatiao_duration_read(p, secs, nanos)
    status == UInt32(BadValue) &&
        refuse(path, "not an exact length: [seconds, nanoseconds], a number of seconds, or {secs, nanos}")
    check(status)
    return Length(secs[], nanos[])
end
readlength(n::Node) = @held n readlength(require_node(n), copy(path_of(n)))

# ---- as values ---------------------------------------------------------------------

const LENGTH_SCHEMA = (type = "array",
    prefixItems = [(type = "integer", minimum = 0, maximum = NumberText("18446744073709551615")),
        (type = "integer", minimum = 0, maximum = 999999999)],
    items = false, minItems = 2, maxItems = 2)
const TIME_PATTERN = "^([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\\.[0-9]{1,9})?\$"
const DATETIME_PATTERN = "^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])[Tt]([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\\.[0-9]{1,9})?\$"

function encode_special!(raw::Ptr{GValue}, x::Union{Date,Time,DateTime,Timestamp,Span}, alloc::Ptr{GAlloc})
    build_text!(raw, alloc, totext(x), false)
    return true
end
function encode_special!(raw::Ptr{GValue}, x::Union{Length,Dates.FixedPeriod}, alloc::Ptr{GAlloc})
    l = x isa Length ? x : Length(x)
    build_list!(raw, (l.seconds, l.nanoseconds), alloc, 0)
    return true
end
function encode_special!(raw::Ptr{GValue}, x::Union{Dates.CompoundPeriod,Dates.OtherPeriod}, alloc::Ptr{GAlloc})
    build_text!(raw, alloc, string(Span(x)), false)
    return true
end

function text_at(p::Ptr{GValue}, path, form)
    load_tag(p) == TAG_STRING || refuse(path, "want a $form, got $(tag_name(p))")
    return text_of(unsafe_load(payload_ptr(GString, p)))
end
function read_form(f, p, path, form)
    text = text_at(p, path, form)
    try
        return Some(f(text))
    catch e
        e isa Unparsed && refuse(path, "not a $form: $(e.expected) at byte $(e.at)")
        rethrow()
    end
end
decode_special(::Type{Date}, p, path) = read_form(t -> fromtext(Date, t), p, path, "date")
decode_special(::Type{Time}, p, path) = read_form(t -> fromtext(Time, t), p, path, "time of day")
decode_special(::Type{DateTime}, p, path) = read_form(t -> fromtext(DateTime, t), p, path, "date and time")
decode_special(::Type{Timestamp}, p, path) = read_form(t -> fromtext(Timestamp, t), p, path, "instant")
decode_special(::Type{Span}, p, path) = read_form(t -> fromtext(Span, t), p, path, "span")
decode_special(::Type{Dates.CompoundPeriod}, p, path) =
    read_form(t -> Dates.CompoundPeriod(fromtext(Span, t)), p, path, "span")
decode_special(::Type{Length}, p, path) = Some(readlength(p, path))
function decode_special(::Type{T}, p, path) where {T<:Dates.FixedPeriod}
    ns = Nanosecond(readlength(p, path))
    v = Dates.value(ns) // Dates.value(Nanosecond(T(1)))
    isinteger(v) || refuse(path, "not a whole number of $(T)s")
    return Some(T(Int64(v)))
end

schema_special(::Type{Date}) = (type = "string", format = "date")
schema_special(::Type{Time}) = (type = "string", pattern = TIME_PATTERN)
schema_special(::Type{DateTime}) = (type = "string", pattern = DATETIME_PATTERN)
schema_special(::Type{Timestamp}) = (type = "string", format = "date-time")
schema_special(::Type{<:Union{Span,Dates.CompoundPeriod}}) = (type = "string", format = "duration")
schema_special(::Type{<:Union{Length,Dates.FixedPeriod}}) = LENGTH_SCHEMA
