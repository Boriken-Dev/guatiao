# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A kind's function table, from Julia: calling one (a remote) and serving
# one (a table of @cfunction shims over Julia objects). `@kind` writes,
# per slot, the caller's method and the callee's shim with the slot's own
# C signature, so every call is a direct ccall with the header's types.

"The supertype of every kind's abstract type."
abstract type Kind end

"""
    Lent{T}

An argument lent for the call (`&mut` in Rust): `Lent{MapRef}`,
`Lent{ListRef}`, `Lent{ValueRef}` (the caller's tree, writable for the
call), `Lent{Vector{UInt8}}` (a buffer the method writes) or an object
kind, `Lent{Listener}` (valid for the call).
"""
struct Lent{T} end

"""
    Taken{T}

An argument handed over with the call: `Taken{MapRef}`,
`Taken{ListRef}`, `Taken{ValueRef}` (the method receives a `Value` it
owns) or an object kind, `Taken{Listener}` (the method owns it and closes
it).
"""
struct Taken{T} end

"Marks a slot every table fills: one the kind was born with, which its floor hash covers."
macro required(ex)
    return esc(Expr(:block, :__guatiao_required__, ex))
end

"Marks a slot that can fail: its C prototype ends with a `guatiao_provider_error *err`."
macro fallible(ex)
    return esc(Expr(:block, :__guatiao_fallible__, ex))
end

"What crosses one argument or answer of a slot."
struct Shape
    kind::Symbol        # :str :bytes :bytes_mut :scalar :value :map :list :object :text
    mode::Symbol        # :borrow :lend :take :answer
    type::Any           # the scalar type, or the object kind's abstract type
end

struct SlotSpec
    name::Symbol
    args::Vector{Shape}
    answer::Union{Nothing,Shape}
    fallible::Bool
    required::Bool
end

"""
A kind described: its C name, floor hash, whether it is an object kind,
its slots in the header's order, the table's size and the bytes its
required slots need.
"""
struct KindSpec
    name::String
    floor_hash::UInt32
    object::Bool
    slots::Vector{SlotSpec}
    size::Int
    floor::Int
end

const WORD = sizeof(Ptr{Cvoid})
first_slot(spec::KindSpec) = sizeof(GKindHeader) + (spec.object ? WORD : 0)
slot_offset(spec::KindSpec, i::Int) = first_slot(spec) + (i - 1) * WORD

function KindSpec(name::String, floor_hash::Integer, object::Bool, slots::Vector{SlotSpec})
    size = sizeof(GKindHeader) + (object ? WORD : 0) + length(slots) * WORD
    # Every slot: a smaller table is refused.
    return KindSpec(name, UInt32(floor_hash), object, slots, size, size)
end

"""
    kind(K) -> KindSpec

The description `@kind` built for the kind `K`.
"""
function kind end

"""
    remote_type(K)

The concrete type a caller holds for the kind `K` (`GreeterRemote` for
`Greeter`): a subtype of `K` with a method of each of its functions.
"""
function remote_type end

slot_pointers(::Type) = nothing   # @kind adds a method per kind
destroy_pointer(::Type) = C_NULL

# ---- a call's scope ------------------------------------------------------------

"""
What one call keeps alive and what runs when it returns: views handed to
a callee check it on every access, so one kept past the call throws.
"""
mutable struct Scope
    open::Bool
    keep::Vector{Any}
    after::Vector{Any}
    given::Vector{Any}   # undone by close unless the call happened
    blocks::Vector{Ptr{Cvoid}}   # C memory released last, after every `after` and `given`
    taken::Vector{Any}           # what a served call was handed: closed if it refuses
    lent::Vector{Tuple{Ptr{GValue},Ptr{GAlloc},Bool}}   # lent trees, each with the allocator it recorded when lent, and whether a map
end
Scope() = Scope(true, Any[], Any[], Any[], Ptr{Cvoid}[], Any[], Tuple{Ptr{GValue},Ptr{GAlloc},Bool}[])
own!(s::Scope, p::Ptr) = (push!(s.blocks, Ptr{Cvoid}(p)); p)
keep!(s::Scope, x) = (push!(s.keep, x); x)
after!(f, s::Scope) = (push!(s.after, f); nothing)
given!(f, s::Scope) = (push!(s.given, f); nothing)
called!(s::Scope) = (empty!(s.given); nothing)
check_scope(s::Scope) = s.open || throw(InvalidStateException("this view was lent for one call, which has returned", :returned))

"""
Moves every tree lent for the call into the allocator it recorded when it
was lent, so what the callee grew it with, or wrote over it, outlives the
callee's library; a lent map also loses any key the callee wrote that is
not UTF-8 (`BadValue`). The first refusal's status, else 0; a tree whose
move is refused is left whole, where it stands.
"""
function settle_lent!(s::Scope)
    status = UInt32(0)
    for (node, home, is_map) in s.lent
        moved = is_map && checks_keys() ? guatiao_map_settle_lent(node, home) : guatiao_value_make_local(node, home)
        moved != 0 && status == 0 && (status = UInt32(moved))
    end
    empty!(s.lent)
    return status
end

"A refused settle after a call that answered: what it answered into `out` freed unread, and the refusal thrown."
function settled!(status::UInt32, out, where::String)
    status == 0 && return nothing
    drop_answer(out)
    status == UInt32(BadValue) &&
        throw(GuatiaoError(status, "$where wrote a key that is not UTF-8 into the map it was lent; the entry was removed"))
    throw(GuatiaoError(status, "moving what $where put into a lent tree into that tree's allocator"))
end
drop_answer(_) = nothing
drop_answer(out::Ref{GValue}) = drop_root(out[])
drop_answer(out::Ref{GMap}) = drop_root(node_of(TAG_MAP, out[]))
drop_answer(out::Ref{GList}) = drop_root(node_of(TAG_LIST, out[]))
drop_answer(out::Ref{GString}) = drop_root(node_of(TAG_STRING, out[]))
drop_answer(out::Ref{GObject}) = destroy_object(out[])
function drop_root(node::GValue)
    root = new_root()
    unsafe_store!(root, node)
    free_root(root)
end

function close_scope(s::Scope)
    s.open || return nothing
    s.open = false
    failure = nothing
    settle_lent!(s)
    for f in Iterators.reverse(s.after)
        try
            f()
        catch e
            failure === nothing && (failure = e)
        end
    end
    for f in Iterators.reverse(s.given)
        try
            f()
        catch e
            failure === nothing && (failure = e)
        end
    end
    foreach(Libc.free, s.blocks)
    empty!(s.blocks)
    empty!(s.keep)
    failure === nothing || throw(failure)
    return nothing
end

# ---- held Julia objects: what a ctx names ---------------------------------------------

# A ctx is a number shaped like an address, above the loader's low bound
# and 16-aligned, never reused: a call naming one released answers GONE.
const HELD_LOCK = Threads.SpinLock()
const HELD = Dict{UInt,Any}()
const NEXT_HELD = Threads.Atomic{UInt}(0)

struct GoneError <: Exception
    status::Status
end

function hold(obj)
    id = Threads.atomic_add!(NEXT_HELD, UInt(1)) + UInt(1)
    ctx = UInt(0x10000) + UInt(16) * id
    lock(HELD_LOCK)
    try
        HELD[ctx] = obj
    finally
        unlock(HELD_LOCK)
    end
    return Ptr{Cvoid}(ctx)
end

function release(ctx::Ptr{Cvoid})
    lock(HELD_LOCK)
    try
        return pop!(HELD, UInt(ctx), nothing)
    finally
        unlock(HELD_LOCK)
    end
end

function held(ctx::Ptr{Cvoid})
    ctx == C_NULL && throw(GoneError(NullArgument))
    lock(HELD_LOCK)
    found = try
        get(HELD, UInt(ctx), HELD)
    finally
        unlock(HELD_LOCK)
    end
    found === HELD && throw(GoneError(Gone))
    return found
end

"""
    live_objects() -> Int

How many Julia objects a C caller can still reach through a ctx: objects
and instances handed out and not yet destroyed or released, and each
provider's one instance.
"""
live_objects() = (lock(HELD_LOCK); try length(HELD) finally unlock(HELD_LOCK) end)

# ---- statuses for refusals ---------------------------------------------------------------

"Reports what a slot that cannot fail failed with; `Guatiao.WARN[] = f` replaces it."
const WARN = Ref{Any}(msg -> (Core.println(Core.stderr, "guatiao: ", msg); nothing))

function status_of(e)
    e isa GoneError && return UInt32(e.status), e.status == Gone ? "the object this call names is gone" : "no object named"
    e isa GuatiaoError && return e.code, e.message
    e isa GuatiaoException && return UInt32(status(e)), sprint(showerror, e)
    return UInt32(Internal), sprint(showerror, e)
end

"Fills a `guatiao_provider_error *`, its message text the caller frees."
function write_err(err::Ptr{GProviderError}, status::UInt32, message::AbstractString)
    err == C_NULL && return nothing
    text = NO_STRING
    if !isempty(message)
        try
            root = new_root()
            try
                build_text!(root, binding_alloc(), message, false)
                text = arm_of(GString, unsafe_load(root))
            catch
                guatiao_value_free(root)
                text = NO_STRING
            finally
                Libc.free(root)
            end
        catch
            text = NO_STRING
        end
    end
    unsafe_store!(err, GProviderError(status, text))
    return nothing
end

"Runs a served slot's body: every exception becomes a status, never crosses into C."
function serve_call(body, where::String, err::Ptr{GProviderError}, fallible::Bool)
    scope = Scope()
    status = UInt32(0)
    try
        body(scope)
    catch e
        status = refuse(e, where, err, fallible)
        # A refusal still owns what it was handed, and frees it now rather than at a collection.
        for t in scope.taken
            try
                close(t)
            catch
            end
        end
    end
    try
        close_scope(scope)
    catch e
        status == 0 && (status = refuse(e, where, err, fallible))
    end
    return status
end

function refuse(e, where, err, fallible)
    s, message = status_of(e)
    s == 0 && (s = UInt32(Internal))
    if fallible && err != C_NULL
        write_err(err, s, message)
    else
        try
            WARN[]("$where cannot fail and failed: $message")
        catch
        end
    end
    return s
end

# ---- the caller's side: passing arguments ------------------------------------------------

function pass_str(x, scope::Scope)
    s = keep!(scope, String(x))
    return gstr(s)
end
function pass_bytes(x, scope::Scope)
    b = keep!(scope, Vector{UInt8}(x))
    return GBytes(isempty(b) ? Ptr{UInt8}(C_NULL) : pointer(b), length(b))
end
function pass_bytes_mut(x::Vector{UInt8}, scope::Scope)
    keep!(scope, x)
    return GBytesMut(isempty(x) ? Ptr{UInt8}(C_NULL) : pointer(x), length(x))
end
pass_bytes_mut(x, ::Scope) = throw(ArgumentError("a mutable bytes argument is a Vector{UInt8} the callee writes into"))
pass_scalar(::Type{T}, x) where {T} = convert(T, x)

const TREE_TAG = Dict(:map => TAG_MAP, :list => TAG_LIST)

"The struct that crosses inside `node`: the node itself for a value, its payload arm for a map or a list."
function arm_ptr(node::Ptr{GValue}, kind::Symbol, where)
    kind === :value && return node
    load_tag(node) == TREE_TAG[kind] || throw(GuatiaoError(WrongKind, "$where: want a $kind, got $(tag_name(node))"))
    return node + PAYLOAD
end

function pass_borrow(x, scope::Scope, kind::Symbol)
    kind === :value && x === nothing && return Ptr{GValue}(C_NULL)
    if x isa Node
        keep!(scope, owner_of(x))
        return arm_ptr(require_node(x), kind, "a borrowed argument")
    end
    v = keep!(scope, Value(x))
    after!(() -> close(v), scope)
    return arm_ptr(root_node(v), kind, "a borrowed argument")
end

function pass_lent(x, scope::Scope, kind::Symbol)
    x isa Node || throw(ArgumentError("a lent argument is a Value or a view, so what the callee changes has somewhere to go"))
    writable_alloc(owner_of(x))
    keep!(scope, owner_of(x))
    node = require_node(x)
    # What the callee grew it with may be in its library's allocator: moved
    # into the one the tree records now, which a callee writing a fresh
    # tree over it would change.
    makes_local() && push!(scope.lent, (node, recorded_alloc(node), kind === :map))
    return arm_ptr(node, kind, "a lent argument")
end

"A root holding `x`'s tree, given up by whoever held it: a Value is moved, a view copied, other data built."
function owned_tree(x, kind::Symbol)
    if x isa ValueRef && isempty(x.path) && x.owner isa Value
        x = x.owner
    end
    root = if x isa Value
        GC.@preserve x begin
            t = load_tag(root_node(x))
            kind === :value || t == TREE_TAG[kind] || throw(GuatiaoError(WrongKind, "a handed-over argument: want a $kind"))
        end
        take_root!(x)
    else
        v = x isa Node ? copy(x) : Value(x)
        GC.@preserve v begin
            t = load_tag(root_node(v))
            if !(kind === :value || t == TREE_TAG[kind])
                close(v)
                throw(GuatiaoError(WrongKind, "a handed-over argument: want a $kind"))
            end
        end
        take_root!(v)
    end
    return root
end

function pass_taken(x, scope::Scope, kind::Symbol)
    # The root struct is ours on every path; what it holds is the callee's once called.
    root = own!(scope, owned_tree(x, kind))
    given!(() -> guatiao_value_free(root), scope)
    return arm_ptr(root, kind, "a handed-over argument")
end

function carrier(kind::Symbol)
    alloc = binding_alloc()
    kind === :map && return Ref(GMap(C_NULL, 0, 0, alloc))
    kind === :list && return Ref(GList(C_NULL, 0, 0, alloc))
    return Ref(node_of(TAG_ABSENT, GMap(C_NULL, 0, 0, alloc)))
end

"Whether the core moves a tree home: a library older than `guatiao_value_make_local` leaves one as a slot built it."
function makes_local()
    found = MAKES_LOCAL[]
    found >> 1 == GENERATION[] && return isodd(found)
    answer = exports("guatiao", "guatiao_value_make_local")
    MAKES_LOCAL[] = GENERATION[] << 1 | Int(answer)
    return answer
end
"`makes_local()`'s answer in the low bit, above the `GENERATION` it was found in: one word, read whole."
const MAKES_LOCAL = Ref(-2)

"Whether the core settles a lent map, removing a key the callee wrote that is not UTF-8."
function checks_keys()
    found = CHECKS_KEYS[]
    found >> 1 == GENERATION[] && return isodd(found)
    answer = exports("guatiao", "guatiao_map_settle_lent")
    CHECKS_KEYS[] = GENERATION[] << 1 | Int(answer)
    return answer
end
"`checks_keys()`'s answer, kept as `MAKES_LOCAL` keeps its own."
const CHECKS_KEYS = Ref(-2)

function answer_tree(out::Ref, kind::Symbol)
    root = new_root()
    unsafe_store!(root, kind === :value ? out[] : node_of(TREE_TAG[kind], out[]))
    if makes_local()
        # A slot that ignored the carrier built it in its own allocator; moved home, it outlives the library.
        status = guatiao_value_make_local(root, binding_alloc())
        if status != 0
            free_root(root)
            throw(GuatiaoError(status, "moving the answer into this binding's allocator"))
        end
    end
    return adopt(root)
end

function answer_text(out::Ref{GString})
    s = out[]
    root = new_root()
    unsafe_store!(root, node_of(TAG_STRING, s))
    try
        text = text_of(s)
        isvalid(String, text) || throw(GuatiaoError(BadValue, "the answer is not UTF-8"))
        return text
    finally
        free_root(root)
    end
end

function finish_call(status::UInt32, err::Ref{GProviderError}, fallible::Bool, where::String)
    status == 0 && return nothing
    message = ""
    if fallible
        e = err[]
        message = text_of(e.message)
        root = new_root()
        unsafe_store!(root, node_of(TAG_STRING, e.message))
        free_root(root)
    else
        message = "$where failed, which its kind says it cannot"
    end
    throw(GuatiaoError(status, message))
end

# ---- the callee's side: receiving arguments ------------------------------------------------

function receive_str(a::GStr)
    s = text_of(a)
    isvalid(String, s) || throw(GuatiaoError(BadValue, "the text argument is not UTF-8"))
    return s
end
receive_bytes(a::GBytes) = a.len == 0 ? UInt8[] : copy(unsafe_wrap(Vector{UInt8}, a.ptr, a.len))
function receive_bytes_mut(a::GBytesMut, scope::Scope)
    # A copy the method writes into, copied back once it returns: an array
    # over the caller's memory would outlive the call if kept.
    a.len == 0 && return UInt8[]
    buf = copy(unsafe_wrap(Vector{UInt8}, a.ptr, a.len))
    after!(scope) do
        n = min(length(buf), Int(a.len))
        GC.@preserve buf unsafe_copyto!(a.ptr, pointer(buf), n)
    end
    return buf
end

"A node of this call's own holding a copy of the map or list struct at `p`."
function node_over(p::Ptr, kind::Symbol, scope::Scope)
    node = own!(scope, Ptr{GValue}(Libc.calloc(1, sizeof(GValue))))
    T = kind === :map ? GMap : GList
    unsafe_store!(node, node_of(TREE_TAG[kind], unsafe_load(Ptr{T}(p))))
    return node
end

wrap_ref(owner, kind::Symbol) = kind === :map ? MapRef(owner, Key[]) : kind === :list ? ListRef(owner, Key[]) : ValueRef(owner, Key[])

function receive_borrowed(p::Ptr, scope::Scope, kind::Symbol)
    if p == C_NULL
        kind === :value && return nothing
        throw(GuatiaoError(NullArgument, "the $kind argument is null"))
    end
    node = kind === :value ? Ptr{GValue}(p) : node_over(p, kind, scope)
    return wrap_ref(View(node; scope), kind)
end

"The allocator a container node records, when complete, else this binding's."
function recorded_alloc(node::Ptr{GValue})
    t = load_tag(node)
    a = t in (TAG_MAP, TAG_LIST, TAG_STRING, TAG_NUMBER, TAG_BYTES) ? unsafe_load(Ptr{Ptr{GAlloc}}(node + PAYLOAD + OWNED_ALLOC)) : Ptr{GAlloc}(C_NULL)
    return complete_alloc(a) ? a : binding_alloc()
end
function complete_alloc(a::Ptr{GAlloc})
    a == C_NULL && return false
    g = unsafe_load(a)
    return g.struct_size >= sizeof(GAlloc) && g.alloc != C_NULL && g.free != C_NULL
end

function receive_lent(p::Ptr, scope::Scope, kind::Symbol)
    p == C_NULL && throw(GuatiaoError(NullArgument, "the lent $kind is null"))
    if kind === :value
        node = Ptr{GValue}(p)
    else
        # What the method changes lands in this node, and is written back once it returns.
        node = node_over(p, kind, scope)
        T = kind === :map ? GMap : GList
        dst = Ptr{T}(p)
        after!(() -> unsafe_store!(dst, unsafe_load(Ptr{T}(node + PAYLOAD))), scope)
    end
    return wrap_ref(View(node; scope, alloc = recorded_alloc(node)), kind)
end

function receive_taken(p::Ptr, kind::Symbol, scope::Scope)
    p == C_NULL && throw(GuatiaoError(NullArgument, "the handed-over $kind is null"))
    root = new_root()
    T = kind === :map ? GMap : kind === :list ? GList : GValue
    unsafe_store!(root, kind === :value ? unsafe_load(Ptr{GValue}(p)) : node_of(TREE_TAG[kind], unsafe_load(Ptr{T}(p))))
    # The method owns it and may keep it: moved out of the caller's
    # allocators, which may go with the caller's library.
    if makes_local()
        status = guatiao_value_make_local(root, binding_alloc())
        if status != 0
            free_root(root)
            throw(GuatiaoError(status, "moving the handed-over $kind into this binding's allocator"))
        end
    end
    v = adopt(root)
    push!(scope.taken, v)
    return v
end

function receive_object_taken(::Type{K}, g::GObject, scope::Scope) where {K}
    r = owned_object(K, g)
    push!(scope.taken, r)
    return r
end

# ---- the callee's side: writing answers -------------------------------------------------------

function write_tree(out::Ptr, result, kind::Symbol)
    out == C_NULL && throw(GuatiaoError(NullArgument, "no slot for the answer"))
    named = kind === :value ? unsafe_load(Ptr{Ptr{GAlloc}}(out + PAYLOAD + OWNED_ALLOC)) : unsafe_load(Ptr{Ptr{GAlloc}}(out + OWNED_ALLOC))
    alloc = complete_alloc(named) ? named : binding_alloc()
    whole = result isa ValueRef && isempty(result.path) && result.owner isa Value ? result.owner : result
    root = if whole isa Value && whole.alloc == alloc
        owned_tree(whole, kind)   # already where the caller wants it: moved
    else
        # Built, or copied, in the allocator the caller named; a Value answered is the callee's to give up.
        r = new_root()
        try
            encode!(r, result, alloc)
        catch
            Libc.free(r)
            rethrow()
        end
        whole isa Value && close(whole)
        if !(kind === :value || load_tag(r) == TREE_TAG[kind])
            free_root(r)
            throw(GuatiaoError(WrongKind, "the answer is not a $kind"))
        end
        r
    end
    try
        if kind === :value
            unsafe_store!(Ptr{GValue}(out), unsafe_load(root))
        else
            T = kind === :map ? GMap : GList
            unsafe_store!(Ptr{T}(out), unsafe_load(Ptr{T}(root + PAYLOAD)))
        end
    finally
        Libc.free(root)
    end
    return nothing
end

function write_text(out::Ptr{GString}, result)
    out == C_NULL && throw(GuatiaoError(NullArgument, "no slot for the answer"))
    result isa AbstractString || throw(ArgumentError("a text answer is a String, not $(typeof(result))"))
    root = new_root()
    try
        build_text!(root, binding_alloc(), result, false)
        unsafe_store!(out, arm_of(GString, unsafe_load(root)))
    finally
        Libc.free(root)
    end
    return nothing
end

function write_scalar(out::Ptr{T}, result) where {T}
    out == C_NULL && throw(GuatiaoError(NullArgument, "no slot for the answer"))
    unsafe_store!(out, convert(T, result))
    return nothing
end

# ---- remotes ---------------------------------------------------------------------------------

"""
The fields every remote carries: the table, its size, the ctx calls name,
what keeps the table valid (`keep`, checked open on every call), the
scope of a lent object, and whether this side owns an object to destroy.
"""
mutable struct RemoteState
    table::Ptr{Cvoid}
    size::Int
    ctx::Ptr{Cvoid}
    keep::Any
    scope::Union{Nothing,Scope}
    owned::Bool
    open::Bool
end

check_keep(::Nothing) = nothing
check_keep(s::Scope) = check_scope(s)
check_keep(x) = isopen(x) || throw(closed_error(x isa Registry ? "registry" : "instance"))

function check_table(spec::KindSpec, table::Ptr{Cvoid}, size::Integer)
    table == C_NULL && throw(FloorMismatch("no $(spec.name) table"))
    size < spec.floor && throw(FloorMismatch("the $(spec.name) table is $size bytes, smaller than the kind's $(spec.floor)"))
    h = unsafe_load(Ptr{GKindHeader}(table))
    h.struct_size < spec.floor &&
        throw(FloorMismatch("the $(spec.name) table declares $(h.struct_size) bytes, smaller than the kind's $(spec.floor)"))
    h.floor_hash == spec.floor_hash ||
        throw(FloorMismatch("the table's floor_hash $(h.floor_hash) is not the $(spec.name) kind's $(spec.floor_hash)"))
    return nothing
end

remote_state(r) = getfield(r, :state)

"The slot's function pointer, or an `Unsupported` error when the provider left it null."
function slot_pointer(r, spec::KindSpec, i::Int)
    st = remote_state(r)
    st.open || throw(closed_error("object"))
    st.scope === nothing || check_scope(st.scope)
    check_keep(st.keep)
    f = unsafe_load(Ptr{Ptr{Cvoid}}(st.table + slot_offset(spec, i)))
    f == C_NULL && throw(GuatiaoError(Unsupported, "this $(spec.name) table has no `$(spec.slots[i].name)`"))
    return f, st.ctx
end

"""
    has(r, slot::Symbol) -> Bool

Whether a remote's table fills the slot (a provider may leave an
optional one null).
"""
function has(r, slot::Symbol)
    st = remote_state(r)
    spec = kind(supertype(typeof(r)))
    i = findfirst(s -> s.name === slot, spec.slots)
    i === nothing && throw(ArgumentError("the $(spec.name) kind has no slot `$slot`"))
    return unsafe_load(Ptr{Ptr{Cvoid}}(st.table + slot_offset(spec, i))) != C_NULL
end

"""
    bind(K, table, size, ctx; keep = nothing)

A caller's handle on any table of the kind `K`: its remote. A
`FloorMismatch` when the table is smaller than the kind's
or has another floor hash. `keep` is what keeps the table valid (it is
checked open on every call).
"""
function bind(::Type{K}, table::Ptr{Cvoid}, size::Integer, ctx::Ptr{Cvoid}; keep = nothing) where {K<:Kind}
    spec = kind(K)
    check_table(spec, table, size)
    return remote_type(K)(RemoteState(table, Int(size), ctx, keep, nothing, false, true))
end

"An object's `destroy` slot, the first after its header."
function destroy_object(g::GObject)
    g.table == C_NULL && return nothing
    g.size >= sizeof(GKindHeader) + WORD || return nothing
    f = unsafe_load(Ptr{Ptr{Cvoid}}(g.table + sizeof(GKindHeader)))
    f == C_NULL && return nothing
    state = gc_safe_enter()
    ccall(f, Cvoid, (Ptr{Cvoid},), g.ctx)
    gc_safe_leave(state)
    return nothing
end

"An object this side owns: closing it calls its destroy slot, once."
function owned_object(::Type{K}, g::GObject) where {K}
    # A table that is not the kind's is refused and the object left alone:
    # its slot after the header need not be a destroy.
    check_table(kind(K), g.table, g.size)
    r = remote_type(K)(RemoteState(g.table, Int(g.size), g.ctx, nothing, nothing, true, true))
    finalizer(close_remote, r)
    return r
end

function close_remote(r)
    st = remote_state(r)
    st.open || return nothing
    st.open = false
    st.owned && destroy_object(GObject(st.table, st.size, st.ctx))
    return nothing
end

"Gives up an owned object: the table and ctx, which the receiver now owns."
function release_object(r)
    st = remote_state(r)
    st.open || throw(closed_error("object"))
    st.owned || throw(ArgumentError("only an object this side owns can be handed over"))
    st.open = false
    return GObject(st.table, st.size, st.ctx)
end

# ---- serving Julia objects -----------------------------------------------------------------------

"What an object or instance held for a C caller carries: the Julia value and its library, if any."
struct Served
    value::Any
    library::Any
end
served_value(x) = x isa Served ? x.value : x

"Whether `T` has a method of the slot's function, taking what the slot receives."
function implements(f, ::Type{T}, slot::SlotSpec) where {T}
    types = Any[T]
    for s in slot.args
        push!(types, receive_type(s))
    end
    return hasmethod(f, Tuple{types...})
end

function receive_type(s::Shape)
    s.kind === :str && return String
    s.kind in (:bytes, :bytes_mut) && return Vector{UInt8}
    s.kind === :scalar && return s.type
    s.kind === :object && return remote_type(s.type)
    s.mode === :take && return Value
    return s.kind === :map ? MapRef : s.kind === :list ? ListRef : ValueRef
end

slot_function(::Type{K}, i::Int) where {K} = kind_functions(K)[i]
kind_functions(::Type) = ()

const TABLES_LOCK = Threads.SpinLock()
const TABLES = Dict{Tuple{DataType,DataType},Ptr{Cvoid}}()

"""
The table serving the kind `K` through objects of type `T`, built on first
use and kept for the process: a shim for each slot `T` has a method for,
null for the rest. A required slot `T` has no method for is an error.
"""
function table_for(::Type{K}, ::Type{T}) where {K<:Kind,T}
    key = (K, T)
    lock(TABLES_LOCK)
    found = try
        get(TABLES, key, C_NULL)
    finally
        unlock(TABLES_LOCK)
    end
    found == C_NULL || return found
    spec = kind(K)
    fns = kind_functions(K)
    missing = [s.name for (i, s) in enumerate(spec.slots) if s.required && !implements(fns[i], T, s)]
    isempty(missing) || throw(ArgumentError("$(T) has no method of $(join(missing, ", ")), which the $(spec.name) kind requires"))
    pointers = slot_pointers(K)
    table = Libc.calloc(1, spec.size)
    unsafe_store!(Ptr{GKindHeader}(table), GKindHeader(spec.size, spec.floor_hash))
    spec.object && unsafe_store!(Ptr{Ptr{Cvoid}}(table + sizeof(GKindHeader)), destroy_pointer(K))
    for (i, s) in enumerate(spec.slots)
        implements(fns[i], T, s) && unsafe_store!(Ptr{Ptr{Cvoid}}(table + slot_offset(spec, i)), pointers[i])
    end
    lock(TABLES_LOCK)
    try
        existing = get(TABLES, key, C_NULL)
        if existing != C_NULL
            Libc.free(table)
            return existing
        end
        TABLES[key] = table
    finally
        unlock(TABLES_LOCK)
    end
    return table
end

"`obj` as a `guatiao_object` of `K`: held until its holder destroys it, or, lent, until `scope` closes."
function serve_object(::Type{K}, obj, scope::Union{Nothing,Scope}) where {K}
    table = table_for(K, typeof(obj))
    ctx = hold(obj)
    scope === nothing || after!(() -> release(ctx), scope)
    return GObject(table, kind(K).size, ctx)
end

function pass_object_taken(::Type{K}, x, scope::Scope) where {K}
    g = x isa remote_type(K) ? release_object(x) : serve_object(K, x, nothing)
    given!(() -> destroy_object(g), scope)
    return g
end

function pass_object_lent(::Type{K}, x, scope::Scope) where {K}
    g = if x isa remote_type(K)
        st = remote_state(x)
        st.open || throw(closed_error("object"))
        GObject(st.table, st.size, st.ctx)
    else
        serve_object(K, x, scope)
    end
    r = keep!(scope, Ref(g))
    return Base.unsafe_convert(Ptr{GObject}, r)
end

answer_object(::Type{K}, out::Ref{GObject}) where {K} = owned_object(K, out[])

function receive_object_lent(::Type{K}, p::Ptr{GObject}, scope::Scope) where {K}
    p == C_NULL && throw(GuatiaoError(NullArgument, "no object was lent"))
    g = unsafe_load(p)
    check_table(kind(K), g.table, g.size)
    return remote_type(K)(RemoteState(g.table, Int(g.size), g.ctx, nothing, scope, false, true))
end

function write_object(::Type{K}, out::Ptr{GObject}, result) where {K}
    out == C_NULL && throw(GuatiaoError(NullArgument, "no slot for the answer"))
    g = result isa remote_type(K) ? release_object(result) : serve_object(K, result, nothing)
    unsafe_store!(out, g)
    return nothing
end

"The destroy slot of every served object: lets the Julia value go, closing it when it can be."
function destroy_served(ctx::Ptr{Cvoid})
    try
        obj = release(ctx)
        v = served_value(obj)
        v === nothing || !hasmethod(close, Tuple{typeof(v)}) || close(v)
    catch e
        try
            WARN[]("destroy failed: $(sprint(showerror, e))")
        catch
        end
    end
    return nothing
end

# ---- @kind -------------------------------------------------------------------------------------------

const SCALAR_NAMES = Dict(:Bool => Bool, :Int8 => Int8, :Int16 => Int16, :Int32 => Int32, :Int64 => Int64,
    :UInt8 => UInt8, :UInt16 => UInt16, :UInt32 => UInt32, :UInt64 => UInt64, :Int => Int, :UInt => UInt,
    :Float32 => Float32, :Float64 => Float64)
const TREE_NAMES = Dict(:ValueRef => :value, :MapRef => :map, :ListRef => :list)
const C_TREE = Dict(:value => GValue, :map => GMap, :list => GList)

macro_name(x) = x isa Symbol ? string(x) : x isa Expr && x.head === :. && x.args[end] isa QuoteNode ? string(x.args[end].value) : string(x)

# A shape as the macro reads it: (kind, mode, scalar type or object kind expression).
function read_shape(ex, answer::Bool, where)
    if ex isa Expr && ex.head === :curly && ex.args[1] in (:Lent, :Taken)
        mode = ex.args[1] === :Lent ? :lend : :take
        answer && error("$where: an answer is neither lent nor taken")
        inner = ex.args[2]
        if inner == :(Vector{UInt8})
            mode === :lend || error("$where: bytes are handed over as Vector{UInt8}, or lent to be written")
            return (:bytes_mut, :lend, nothing)
        end
        haskey(TREE_NAMES, inner) && return (TREE_NAMES[inner], mode, nothing)
        return (:object, mode, inner)
    end
    if answer
        ex === :Nothing && return nothing
        ex === :String && return (:text, :answer, nothing)
        haskey(SCALAR_NAMES, ex) && return (:scalar, :answer, SCALAR_NAMES[ex])
        haskey(TREE_NAMES, ex) && return (TREE_NAMES[ex], :answer, nothing)
        return (:object, :answer, ex)
    end
    ex === :String && return (:str, :borrow, nothing)
    ex == :(Vector{UInt8}) && return (:bytes, :borrow, nothing)
    haskey(SCALAR_NAMES, ex) && return (:scalar, :borrow, SCALAR_NAMES[ex])
    ex === :ValueRef && return (:value, :borrow, nothing)
    ex === :MapRef && return (:map, :borrow, nothing)
    ex === :ListRef && error("$where: a list is never borrowed: Lent{ListRef} or Taken{ListRef}")
    error("$where: $(ex) is no shape a slot can carry; an object is Taken{K} or Lent{K}")
end

c_arg_type(sh) = sh[1] === :str ? GStr : sh[1] === :bytes ? GBytes : sh[1] === :bytes_mut ? GBytesMut :
    sh[1] === :scalar ? sh[3] : sh[1] === :object ? (sh[2] === :take ? GObject : Ptr{GObject}) : Ptr{C_TREE[sh[1]]}
c_out_type(sh) = sh[1] === :text ? Ptr{GString} : sh[1] === :scalar ? Ptr{sh[3]} :
    sh[1] === :object ? Ptr{GObject} : Ptr{C_TREE[sh[1]]}

"""
    @kind Name floor_hash = N [name = "c_name"] [object = true] begin
        [@required] [@fallible] slot(arg::Shape, ...)::Answer
        ...
    end

Declares a kind from its C header: `abstract type Name <: Guatiao.Kind`,
a generic function per slot (new functions of the calling module), the
description `Guatiao.kind(Name)`, and `NameRemote <: Name`, whose method of
each function calls the slot. Slots are listed in the header's order and
found by position; `floor_hash` is the header's `<table>_FLOOR_HASH`. An
object kind's `destroy` slot is `close` on its remote. Object kinds a slot
names must be declared before it.
"""
macro kind(name::Symbol, args...)
    body = args[end]
    body isa Expr && body.head === :block || error("@kind takes its slots in a begin ... end block")
    opts = Dict{Symbol,Any}()
    for a in args[1:end-1]
        a isa Expr && a.head === :(=) || error("@kind options are key = value")
        opts[a.args[1]] = a.args[2]
    end
    haskey(opts, :floor_hash) || error("@kind $name needs floor_hash, the header's <table>_FLOOR_HASH")
    cname = string(get(opts, :name, lowercase(string(name))))
    object = get(opts, :object, false) === true
    G = Guatiao
    remote = Symbol(name, :Remote)
    slots = []
    for ex in body.args
        ex isa LineNumberNode && continue
        required = false
        fallible = false
        while ex isa Expr && ex.head === :macrocall
            m = macro_name(ex.args[1])
            if endswith(m, "required")
                required = true
            elseif endswith(m, "fallible")
                fallible = true
            else
                error("@kind $name: unexpected $m before a slot")
            end
            ex = filter(x -> !(x isa LineNumberNode), ex.args[2:end])[1]
        end
        ex isa Expr && ex.head === :(::) || error("@kind $name: a slot is `name(arg::Shape, ...)::Answer`")
        call, ans = ex.args
        fname = call.args[1]
        where = "$name.$fname"
        argnames = Symbol[]
        shapes = []
        for a in call.args[2:end]
            a isa Expr && a.head === :(::) || error("$where: every argument needs a shape")
            push!(argnames, a.args[1])
            push!(shapes, read_shape(a.args[2], false, "$where($(a.args[1]))"))
        end
        push!(slots, (fname = fname, argnames = argnames, shapes = shapes, answer = read_shape(ans, true, "$where answer"),
            fallible = fallible, required = required))
    end

    shape_expr(sh) = sh === nothing ? :nothing : :($G.Shape($(QuoteNode(sh[1])), $(QuoteNode(sh[2])), $(sh[3] === nothing ? :nothing : sh[3])))
    slot_exprs = [:($G.SlotSpec($(QuoteNode(s.fname)), $G.Shape[$(map(shape_expr, s.shapes)...)], $(shape_expr(s.answer)),
        $(s.fallible), $(s.required))) for s in slots]

    spec_const = Symbol("__guatiao_kind_", name)
    out = Expr(:block)
    push!(out.args, :(Core.@__doc__ abstract type $name <: $G.Kind end))
    push!(out.args, :(mutable struct $remote <: $name
        state::$G.RemoteState
    end))
    for f in unique(s.fname for s in slots)
        push!(out.args, :(function $f end))
    end
    push!(out.args, :(const $spec_const = $G.KindSpec($cname, $(opts[:floor_hash]), $object, $G.SlotSpec[$(slot_exprs...)])))
    push!(out.args, :($G.kind(::Type{$name}) = $spec_const))
    push!(out.args, :($G.remote_type(::Type{$name}) = $remote))
    push!(out.args, :($G.kind_functions(::Type{$name}) = ($(map(s -> s.fname, slots)...),)))
    push!(out.args, :(Base.close(r::$remote) = $G.close_remote(r)))
    push!(out.args, :(Base.isopen(r::$remote) = $G.remote_state(r).open))
    push!(out.args, :(Base.show(io::IO, r::$remote) = print(io, $(string(remote)), "(", $cname, ")")))

    shim_names = Symbol[]
    shim_sigs = []
    for (i, s) in enumerate(slots)
        where = "`$cname.$(s.fname)`"
        # Locals of the generated code, named so no argument can shadow them.
        f_, ctx_, scope_, out_, err_, st_, gs_, r_ = (Symbol("__gq_", x) for x in (:f, :ctx, :scope, :out, :err, :st, :gs, :r))
        # ---- the caller's method ----
        cvals = [Symbol("__gq_c", j) for j in eachindex(s.shapes)]
        ctypes = Any[Ptr{Cvoid}]
        for sh in s.shapes
            push!(ctypes, c_arg_type(sh))
        end
        order = vcat([j for (j, sh) in enumerate(s.shapes) if sh[2] !== :take],
            [j for (j, sh) in enumerate(s.shapes) if sh[2] === :take])
        conv = Expr[]
        for j in order
            sh = s.shapes[j]
            a = s.argnames[j]
            rhs = sh[1] === :str ? :($G.pass_str($a, $scope_)) :
                  sh[1] === :bytes ? :($G.pass_bytes($a, $scope_)) :
                  sh[1] === :bytes_mut ? :($G.pass_bytes_mut($a, $scope_)) :
                  sh[1] === :scalar ? :($G.pass_scalar($(sh[3]), $a)) :
                  sh[1] === :object ? (sh[2] === :take ? :($G.pass_object_taken($(sh[3]), $a, $scope_)) :
                                       :($G.pass_object_lent($(sh[3]), $a, $scope_))) :
                  sh[2] === :borrow ? :($G.pass_borrow($a, $scope_, $(QuoteNode(sh[1])))) :
                  sh[2] === :lend ? :($G.pass_lent($a, $scope_, $(QuoteNode(sh[1])))) :
                  :($G.pass_taken($a, $scope_, $(QuoteNode(sh[1]))))
            push!(conv, :($(cvals[j]) = $rhs))
        end
        ans = s.answer
        out_decl = ans === nothing ? :($out_ = nothing) :
            ans[1] === :text ? :($out_ = Ref($G.NO_STRING)) :
            ans[1] === :scalar ? :($out_ = Ref{$(ans[3])}(zero($(ans[3])))) :
            ans[1] === :object ? :($out_ = Ref($G.GObject(C_NULL, 0, C_NULL))) :
            :($out_ = $G.carrier($(QuoteNode(ans[1]))))
        ptr_args = Any[]
        if ans !== nothing
            push!(ctypes, c_out_type(ans))
            push!(ptr_args, :(Base.unsafe_convert($(c_out_type(ans)), $out_)))
        end
        if s.fallible
            push!(ctypes, Ptr{GProviderError})
            push!(ptr_args, :(Base.unsafe_convert(Ptr{$G.GProviderError}, $err_)))
        end
        answer_expr = ans === nothing ? :(nothing) :
            ans[1] === :text ? :($G.answer_text($out_)) :
            ans[1] === :scalar ? :($out_[]) :
            ans[1] === :object ? :($G.answer_object($(ans[3]), $out_)) :
            :($G.answer_tree($out_, $(QuoteNode(ans[1]))))
        pvals = [Symbol("__gq_p", k) for k in eachindex(ptr_args)]
        caller = quote
            function $(s.fname)($r_::$remote, $(s.argnames...))
                $f_, $ctx_ = $G.slot_pointer($r_, $spec_const, $i)
                $scope_ = $G.Scope()
                try
                    $(conv...)
                    $out_decl
                    $err_ = Ref($G.GProviderError(0, $G.NO_STRING))
                    $st_ = GC.@preserve $scope_ $err_ $out_ begin
                        $([:($(pvals[k]) = $(ptr_args[k])) for k in eachindex(ptr_args)]...)
                        $gs_ = $G.gc_safe_enter()
                        local status = ccall($f_, UInt32, ($(ctypes...),), $ctx_, $(cvals...), $(pvals...))
                        $G.gc_safe_leave($gs_)
                        status
                    end
                    $G.called!($scope_)
                    # Answered or refused; the slot's own refusal wins over the settle's.
                    local __gq_settled = $G.settle_lent!($scope_)
                    $G.finish_call($st_, $err_, $(s.fallible), $where)
                    $G.settled!(__gq_settled, $out_, $where)
                    return $answer_expr
                finally
                    $G.close_scope($scope_)
                end
            end
        end
        push!(out.args, caller)

        # ---- the callee's shim ----
        shim = Symbol("__guatiao_shim_", name, "_", s.fname)
        push!(shim_names, shim)
        params = [Symbol("__gq_a", j) for j in eachindex(s.shapes)]
        sig = Any[:($ctx_::Ptr{Cvoid})]
        for (j, sh) in enumerate(s.shapes)
            push!(sig, :($(params[j])::$(c_arg_type(sh))))
        end
        ans !== nothing && push!(sig, :($out_::$(c_out_type(ans))))
        s.fallible && push!(sig, :($err_::Ptr{$G.GProviderError}))
        push!(shim_sigs, ctypes)
        vals = [Symbol("__gq_v", j) for j in eachindex(s.shapes)]
        target_ = Symbol("__gq_target")
        result_ = Symbol("__gq_result")
        recv = Expr[]
        for take_first in (true, false)
            for (j, sh) in enumerate(s.shapes)
                (sh[2] === :take) == take_first || continue
                p = params[j]
                rhs = sh[1] === :str ? :($G.receive_str($p)) :
                      sh[1] === :bytes ? :($G.receive_bytes($p)) :
                      sh[1] === :bytes_mut ? :($G.receive_bytes_mut($p, $scope_)) :
                      sh[1] === :scalar ? p :
                      sh[1] === :object ? (sh[2] === :take ? :($G.receive_object_taken($(sh[3]), $p, $scope_)) :
                                           :($G.receive_object_lent($(sh[3]), $p, $scope_))) :
                      sh[2] === :borrow ? :($G.receive_borrowed($p, $scope_, $(QuoteNode(sh[1])))) :
                      sh[2] === :lend ? :($G.receive_lent($p, $scope_, $(QuoteNode(sh[1])))) :
                      :($G.receive_taken($p, $(QuoteNode(sh[1])), $scope_))
                # Every handed-over argument is taken, even past a refusal, so the call owns all of it.
                push!(recv, take_first ?
                    :($(vals[j]) = try
                        $rhs
                    catch __gq_e
                        __gq_refused === nothing && (__gq_refused = __gq_e)
                        nothing
                    end) :
                    :($(vals[j]) = $rhs))
            end
            # What was handed over is taken first, so a refusal below still owns it and frees it.
            if take_first
                pushfirst!(recv, :(__gq_refused = nothing))
                push!(recv, :(__gq_refused === nothing || throw(__gq_refused)))
                push!(recv, :($target_ = $G.served_value($G.held($ctx_))))
            end
        end
        write_expr = ans === nothing ? :(nothing) :
            ans[1] === :text ? :($G.write_text($out_, $result_)) :
            ans[1] === :scalar ? :($G.write_scalar($out_, $result_)) :
            ans[1] === :object ? :($G.write_object($(ans[3]), $out_, $result_)) :
            :($G.write_tree($out_, $result_, $(QuoteNode(ans[1]))))
        errarg = s.fallible ? err_ : :(Ptr{$G.GProviderError}(C_NULL))
        shim_def = quote
            function $shim($(sig...))::UInt32
                return $G.serve_call($where, $errarg, $(s.fallible)) do $scope_
                    $(recv...)
                    $result_ = $(s.fname)($target_, $(vals...))
                    $write_expr
                    nothing
                end
            end
        end
        push!(out.args, shim_def)
    end
    # The shims' C pointers, taken at run time: a pointer is never kept across sessions.
    ptrs = [:(@cfunction($(shim_names[i]), UInt32, ($(shim_sigs[i]...),))) for i in eachindex(slots)]
    push!(out.args, :($G.slot_pointers(::Type{$name}) = Ptr{Cvoid}[$(ptrs...)]))
    if object
        push!(out.args, :($G.destroy_pointer(::Type{$name}) = @cfunction($G.destroy_served, Cvoid, (Ptr{Cvoid},))))
    end
    push!(out.args, :nothing)
    return esc(out)
end
