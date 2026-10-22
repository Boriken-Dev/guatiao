# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The shared vectors every implementation reads, then the mappings to
# Julia's own types and what they lose.

using Dates
using Guatiao: parse_fields, format_fields, GDate, GDateTime, GSpan

fields_list(g::Guatiao.GDateTime) = [fields_list(g.date); fields_list(g.time)]
fields_list(g) = BigInt[getfield(g, i) for i in 1:fieldcount(typeof(g))]

@testset "every vector of every form" begin
    vecs = vectors()
    for form in Guatiao.FORMS
        cases = ListRef(vecs[string(form)])
        @test length(cases) > 0
        for case in cases
            text = decode(String, case["text"])
            if haskey(MapRef(case), "refused_at")
                at = decode(Int, case["refused_at"])
                for read in (() -> parse_fields(form, text), () -> Guatiao.normalize(form, text))
                    e = try read() catch e; e end
                    @test e isa Unparsed
                    @test e.at == at
                    @test e.form === form
                    @test !isempty(e.expected)
                    length(text) > 2 && @test !occursin(text, sprint(showerror, e))
                end
            else
                g = parse_fields(form, text)
                @test fields_list(g) == decode(Vector{BigInt}, case["fields"])
                @test format_fields(form, g) == decode(String, case["canonical"])
                @test Guatiao.normalize(form, text) == decode(String, case["canonical"])
            end
        end
    end
    close(vecs)
end

@testset "every span applied" begin
    vecs = vectors()
    for (form, T, f) in (("date_add_span", GDate, Guatiao.guatiao_date_add_span),
        ("datetime_add_span", GDateTime, Guatiao.guatiao_datetime_add_span))
        kind = Symbol(split(form, "_")[1])
        for case in ListRef(vecs[form])
            start = Ref(parse_fields(kind, decode(String, case["start"])))
            span = Ref(parse_fields(:span, decode(String, case["span"])))
            out = Ref{T}()
            status = GC.@preserve start span out f(start, span, out)
            if Guatiao.isnull(case["result"])
                @test status == UInt32(Guatiao.BadValue)
            else
                @test status == 0
                @test format_fields(kind, out[]) == decode(String, case["result"])
            end
        end
    end
    close(vecs)
end

@testset "every exact length" begin
    vecs = vectors()
    for case in ListRef(vecs["duration"])
        if Guatiao.isnull(case["duration"])
            @test_throws GuatiaoError Guatiao.readlength(case["value"])
        else
            s, ns = decode(Tuple{UInt64,UInt32}, case["duration"])
            @test Guatiao.readlength(case["value"]) == Length(s, ns)
        end
    end
    close(vecs)
end

@testset "the mappings to Dates" begin
    @test Guatiao.fromtext(Date, "0000-02-29") == Date(0, 2, 29)
    @test Guatiao.totext(Date(2026, 10, 21)) == "2026-10-21"
    t = Guatiao.fromtext(Time, "13:45:00.000000001")
    @test Dates.nanosecond(t) == 1
    @test Guatiao.totext(t) == "13:45:00.000000001"
    # A DateTime counts milliseconds: what is below one is dropped toward the past.
    @test Guatiao.fromtext(DateTime, "2026-10-21T13:45:00.0019") == DateTime(2026, 10, 21, 13, 45, 0, 1)
    @test Guatiao.totext(DateTime(2026, 10, 21, 13, 45, 0, 500)) == "2026-10-21T13:45:00.5"
    ts = Guatiao.fromtext(Timestamp, "2026-10-21T19:15:00.5+05:30")
    @test ts == Timestamp(1792590300, 500000000)
    @test string(ts) == "2026-10-21T13:45:00.5Z"
    @test DateTime(ts) == DateTime(2026, 10, 21, 13, 45, 0, 500)
    @test Timestamp(DateTime(1969, 12, 31, 23, 59, 59, 999)) == Timestamp(-1, 999_000_000)
    @test_throws Unparsed Guatiao.fromtext(Date, "2026-10-21Z")
    @test Guatiao.normalize(:timestamp, "2026-10-21T13:45:00.000000001+00:00") == "2026-10-21T13:45:00.000000001Z"
end

@testset "a span is its own type, backed by the library" begin
    s = Span(days = 1, hours = 2)
    @test string(s) == "P1DT2H"
    @test string(-s) == "-P1DT2H"
    @test Span(days = 1) != Span(hours = 24)
    @test iszero(Span()) && Guatiao.isnegative(-s)
    @test_throws GuatiaoError Span(days = 1, hours = -1)
    @test_throws GuatiaoError Span(nanoseconds = 1_000_000_000)
    @test Guatiao.fromtext(Span, "1h 30m") == Span(hours = 1, minutes = 30)
    @test Span(Hour(1) + Millisecond(1500)) == Span(hours = 1, seconds = 1, nanoseconds = 500_000_000)
    @test Dates.CompoundPeriod(Span(months = 1, days = 2)) == Month(1) + Day(2)
    @test Date(2026, 1, 31) + Span(months = 1) == Date(2026, 2, 28)
    @test Date(2026, 1, 31) + Span(months = 1, days = 1) == Date(2026, 3, 1)
    @test DateTime(2026, 10, 21, 23, 30) + Span(hours = 1) == DateTime(2026, 10, 22, 0, 30)
    @test_throws GuatiaoError Date(2026, 10, 21) + Span(hours = 1)
end

@testset "dates and lengths as values" begin
    Value((d = Date(2026, 10, 21), t = Time(13, 45), dt = DateTime(2026, 10, 21, 13, 45),
        ts = Timestamp(0, 0), s = Span(weeks = 1), l = Length(90, 500_000_000), n = Second(90))) do v
        @test v[] == Dict("d" => "2026-10-21", "t" => "13:45:00", "dt" => "2026-10-21T13:45:00",
            "ts" => "1970-01-01T00:00:00Z", "s" => "P1W", "l" => [90, 500000000], "n" => [90, 0])
        @test decode(Date, v["d"]) == Date(2026, 10, 21)
        @test decode(Timestamp, v["ts"]) == Timestamp(0, 0)
        @test decode(Span, v["s"]) == Span(weeks = 1)
        @test decode(Length, v["l"]) == Length(90, 500_000_000)
        @test decode(Millisecond, v["l"]) == Millisecond(90_500)
        @test_throws GuatiaoError decode(Second, v["l"])
        @test_throws GuatiaoError decode(Date, v["t"])
    end
    @test Nanosecond(Length(1, 5)) == Nanosecond(1_000_000_005)
    @test_throws InexactError Nanosecond(Length(typemax(UInt64), 0))
    @test_throws GuatiaoError Value(Second(-1))
    @test decode(Length, Value(1.5)) == Length(1, 500_000_000)
end
