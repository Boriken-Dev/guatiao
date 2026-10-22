# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# JSON text as plain data a Value takes, for the tests' documents: an
# object is a NamedTuple (its keys in order), a number a NumberText (its
# text exactly). The suite needs no other package to read them.

function read_json(text::AbstractString)
    s = String(text)
    value, i = json_value(s, json_space(s, firstindex(s)))
    json_space(s, i) > ncodeunits(s) || error("JSON: text after the value at byte $i")
    return value
end

json_space(s, i) = (while i <= ncodeunits(s) && codeunit(s, i) in (0x20, 0x09, 0x0a, 0x0d); i += 1; end; i)

function json_value(s, i)
    c = Char(codeunit(s, i))
    c == '{' && return json_object(s, i + 1)
    c == '[' && return json_array(s, i + 1)
    c == '"' && return json_string(s, i + 1)
    startswith(SubString(s, i), "true") && return true, i + 4
    startswith(SubString(s, i), "false") && return false, i + 5
    startswith(SubString(s, i), "null") && return nothing, i + 4
    j = i
    while j <= ncodeunits(s) && Char(codeunit(s, j)) in "+-0123456789.eE"
        j += 1
    end
    j > i || error("JSON: unexpected '$c' at byte $i")
    return NumberText(s[i:j-1]), j
end

function json_object(s, i)
    keys, values = Symbol[], Any[]
    i = json_space(s, i)
    if codeunit(s, i) == UInt8('}')
        return NamedTuple(), i + 1
    end
    while true
        key, i = json_string(s, json_space(s, i) + 1)
        i = json_space(s, i)
        codeunit(s, i) == UInt8(':') || error("JSON: ':' expected at byte $i")
        value, i = json_value(s, json_space(s, i + 1))
        push!(keys, Symbol(key))
        push!(values, value)
        i = json_space(s, i)
        c = codeunit(s, i)
        c == UInt8('}') && return NamedTuple{Tuple(keys)}(Tuple(values)), i + 1
        c == UInt8(',') || error("JSON: ',' or '}' expected at byte $i")
        i += 1
    end
end

function json_array(s, i)
    items = Any[]
    i = json_space(s, i)
    codeunit(s, i) == UInt8(']') && return items, i + 1
    while true
        value, i = json_value(s, json_space(s, i))
        push!(items, value)
        i = json_space(s, i)
        c = codeunit(s, i)
        c == UInt8(']') && return items, i + 1
        c == UInt8(',') || error("JSON: ',' or ']' expected at byte $i")
        i += 1
    end
end

function json_string(s, i)
    out = IOBuffer()
    while true
        c = codeunit(s, i)
        if c == UInt8('"')
            return String(take!(out)), i + 1
        elseif c == UInt8('\\')
            e = Char(codeunit(s, i + 1))
            if e == 'u'
                u = parse(UInt32, s[i+2:i+5]; base = 16)
                i += 6
                if 0xd800 <= u <= 0xdbff && codeunit(s, i) == UInt8('\\')
                    low = parse(UInt32, s[i+2:i+5]; base = 16)
                    u = 0x10000 + ((u - 0xd800) << 10) + (low - 0xdc00)
                    i += 6
                end
                print(out, Char(u))
                continue
            end
            write(out, get(Dict('b' => '\b', 'f' => '\f', 'n' => '\n', 'r' => '\r', 't' => '\t'), e, e))
            i += 2
        else
            write(out, c)
            i += 1
        end
    end
end
