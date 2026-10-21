// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[guatiao::kind]`: a provider kind declared as a trait.
//!
//! The trait is emitted with one provided method added beside each one
//! answering a `Value`, `Map` or `List`: `<method>_in`, the same call
//! naming the allocator the caller wants the answer in, which is what a
//! shim calls (the caller's allocator arrives in the out slot as an empty
//! container, a carrier). Beside the trait:
//!
//! - `<Trait>Vtable`, a `repr(C)` table: a `KindHeader`, then one slot per
//!   method in declaration order, each a monomorphised shim;
//! - `<Trait>Vtable::of::<T>()`, a `const fn` filling the table for an
//!   implementation, and one `<method>_end()` per slot;
//! - `impl Kind for dyn Trait`, naming the table, its floor and its hash;
//! - `impl Trait for Remote<dyn Trait>`, the proxy that calls through a
//!   table it was handed;
//! - `From<Remote<dyn Trait>>` for `Box<dyn Trait>` (`Box` is the one
//!   fundamental type the orphan rule allows; `Kind::shared` makes an `Arc`);
//! - for a provider kind, `impl Trait for Instance<dyn Trait>` and its
//!   `From` for `Box<dyn Trait>`, so a configured instance is stored like
//!   any implementation and released with its last owner.
//!
//! Every `unsafe` the shims and the proxy need is a call into
//! `::guatiao::library::kind`; this file writes tokens and never runs any.
//!
//! # What may cross
//!
//! A method takes `&self` and the trait names `Send + Sync`. Arguments:
//! integers, floats, `bool`, `&str`, `&[u8]`, `&Value`, `Option<&Value>`,
//! `&Map`; `Value`, `Map` and `List` by value, which cross as a move (a
//! pointer to the caller's node, which the callee then owns); `&mut
//! Value`, `&mut Map` and `&mut List`, lent for the call (a pointer to the
//! caller's node, which the callee may change and grow; once it returns,
//! the proxy moves whatever the callee built in another allocator into the
//! one the tree records); and any other type by value, which crosses as a
//! value through `ToValue`. Returns: `()`, the scalars, `Value`, `Map`, `List`, `String`,
//! and any other type through `FromValue` — each optionally inside
//! `Result<_, ProviderError>`. A method converting through `ToValue` or
//! `FromValue` must return a `Result`, because the conversion can fail.
//!
//! # Object kinds
//!
//! `#[guatiao::kind(object)]` declares a kind whose instances are
//! **handles one caller owns** — a session, a scan, a stream — rather
//! than providers a registry offers. The trait names `Send` (not
//! necessarily `Sync`); its methods may take `&mut self`; a `&mut [u8]`
//! argument crosses as an out-buffer (`BytesMut`); and the table carries
//! a `destroy` slot right after the header, which dropping the handle
//! calls. The trait gains `into_object(self) -> Object<dyn Trait>`, the
//! way a Rust implementation becomes a handle. Any kind's method may
//! return `Object<dyn K>` for an object kind `K`, or take one as an
//! argument (ownership crosses with it) — a host implementing an object
//! kind and handing it in is how a callback crosses.
//!
//! Or **lend** one for the call: an argument `&mut dyn K` (or `&dyn K`,
//! when `K` has a `&self` method) crosses as a pointer to an `ObjectRaw`
//! on the caller's stack, its `ctx` the address of the caller's own
//! `*mut dyn K` and its table the kind's lent table (`lent_mut()`,
//! `lent()`), whose slots call through that pointer and whose `destroy`
//! does nothing. The callee gets a `BorrowedMut`/`Borrowed`, which has no
//! `Drop`. Nothing is allocated and nothing is owed back. Lent shared, a
//! slot whose method takes `&mut self` answers `GUATIAO_ERR_UNSUPPORTED`.
//!
//! # Versioning
//!
//! Slots follow declaration order. A method **with a default body** is an
//! appended slot: a table too short to hold it makes the proxy run the
//! default. A required method after a defaulted one is refused, so the
//! floor is one contiguous prefix. The header's hash covers the required
//! methods' names and signatures, so a table built from a different
//! declaration is refused rather than called with the wrong ABI.
//!
//! The signatures are hashed as **spelled**, after one canonical rewrite
//! (`crate::spelling`): `std`/`core`/`alloc` paths of `Vec`, `String`,
//! `Option`, `Result` and `Box`, and `guatiao` paths (or the `crate =`
//! root's) of `Value`, `Map`, `List`, `Text`, `ProviderError`, `Object`
//! and `Status`, become the bare name; a leading `::`, a reference's
//! lifetime and parentheses go. Anything else is hashed as written,
//! because a proc macro has no type information: renaming a model type, or
//! naming it through an alias or another module path, changes the hash
//! even where the wire shape does not.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::{
    Block, FnArg, GenericArgument, Ident, ItemTrait, PathArguments, ReturnType, TraitItem,
    TraitItemFn, Type, TypeReference,
};

use crate::spelling::Roots;

/// Expands `#[kind(...)]` over a trait, or emits a compile error at the
/// user's span.
pub(crate) fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    match try_expand(attr, item) {
        Ok(tokens) => tokens,
        Err(e) => e.to_compile_error(),
    }
}

fn try_expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let tr: ItemTrait = syn::parse2(item)?;
    let attr = parse_attr(attr, &tr.ident)?;
    check_trait(&tr, attr.object)?;
    let methods = plan_methods(&tr, attr.object, Roots::new(attr.krate.as_ref()))?;
    Ok(crate::krate::reroot(
        emit(&tr, &attr, &methods),
        attr.krate.as_ref(),
    ))
}

// --- what the attribute takes ---------------------------------------------

/// What `#[kind(...)]` was given.
struct KindAttr {
    /// The kind's name in a descriptor.
    name: String,
    /// `object`: a handle one caller owns, not a provider.
    object: bool,
    /// `crate = <path>`: where the expansion finds `guatiao`.
    krate: Option<syn::Path>,
}

/// `#[kind]`, or any of `name = "..."`, `object` and `crate = <path>`.
/// The name defaults to the trait's ident in snake case.
fn parse_attr(attr: TokenStream, ident: &Ident) -> syn::Result<KindAttr> {
    let mut name = None;
    let mut object = false;
    let mut krate = None;
    if !attr.is_empty() {
        let parser = syn::meta::parser(|meta| {
            if meta.path.is_ident("name") {
                let lit: syn::LitStr = meta.value()?.parse()?;
                if lit.value().is_empty() {
                    return Err(meta.error("a kind's name cannot be empty"));
                }
                name = Some(lit.value());
                Ok(())
            } else if meta.path.is_ident("object") {
                if meta.input.peek(syn::Token![=]) {
                    return Err(meta.error("`object` takes no value; write `#[kind(object)]`"));
                }
                object = true;
                Ok(())
            } else if meta.path.is_ident("crate") {
                krate = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta
                    .error("#[kind] takes only `name = \"...\"`, `object` and `crate = <path>`"))
            }
        });
        syn::parse::Parser::parse2(parser, attr)?;
    }
    Ok(KindAttr {
        name: name.unwrap_or_else(|| snake_case(&ident.to_string())),
        object,
        krate,
    })
}

/// `Greeter` → `greeter`, `SessionBackend` → `session_backend`.
pub(crate) fn snake_case(ident: &str) -> String {
    let mut out = String::with_capacity(ident.len() + 4);
    let chars: Vec<char> = ident.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev_lower =
                i > 0 && (chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit());
            let next_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
            if i > 0 && (prev_lower || next_lower) {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

// --- what the trait may look like ----------------------------------------

fn check_trait(tr: &ItemTrait, object: bool) -> syn::Result<()> {
    if let Some(u) = &tr.unsafety {
        return Err(syn::Error::new_spanned(
            u,
            "a kind cannot be an `unsafe trait`; the proxy implements it in safe code",
        ));
    }
    if tr.generics.params.iter().next().is_some() {
        return Err(syn::Error::new_spanned(
            &tr.generics,
            "a kind cannot be generic: one trait is one table, and a table has one layout",
        ));
    }
    if let Some(w) = &tr.generics.where_clause {
        return Err(syn::Error::new_spanned(
            w,
            "a kind cannot carry a `where` clause; the proxy implements it for every table",
        ));
    }
    let names_send = tr.supertraits.iter().any(|b| bound_is(b, "Send"));
    let names_sync = tr.supertraits.iter().any(|b| bound_is(b, "Sync"));
    if object {
        if !names_send {
            return Err(syn::Error::new_spanned(
                &tr.ident,
                "an object kind must name `Send` as a supertrait: a handle is handed to another \
                 thread",
            ));
        }
    } else if !(names_send && names_sync) {
        return Err(syn::Error::new_spanned(
            &tr.ident,
            "a kind must name `Send + Sync` as supertraits: a provider is called from any thread",
        ));
    }
    Ok(())
}

fn bound_is(bound: &syn::TypeParamBound, name: &str) -> bool {
    match bound {
        syn::TypeParamBound::Trait(t) => t.path.segments.last().is_some_and(|s| s.ident == name),
        _ => false,
    }
}

// --- the plan for each method ---------------------------------------------

/// How an argument crosses.
#[derive(Clone)]
enum ArgKind {
    /// By value, the same type on both sides.
    Scalar(Type),
    /// `&str`, as a `Str` view.
    Str,
    /// `&[u8]`, as a `Bytes` view.
    Bytes,
    /// `&Value`, as a pointer that must not be null.
    ValueRef,
    /// `Option<&Value>`, as a pointer that may be null.
    ValueOpt,
    /// `&Map`, as a pointer that must not be null.
    MapRef,
    /// `Value` by value: a pointer to the caller's node, which the callee
    /// owns once called and the caller never frees.
    ValueOwned,
    /// `Map` by value, as [`ArgKind::ValueOwned`].
    MapOwned,
    /// `List` by value, as [`ArgKind::ValueOwned`].
    ListOwned,
    /// `&mut Value`: a pointer to the caller's node, lent for the call.
    /// The callee may change and grow it, never free it or keep it.
    ValueMut,
    /// `&mut Map`, as [`ArgKind::ValueMut`].
    MapMut,
    /// `&mut List`, as [`ArgKind::ValueMut`].
    ListMut,
    /// Any other type by value, built into a value with `ToValue` and
    /// read back with `FromValue`.
    Owned(Type),
    /// `&mut [u8]` on an object kind, as a writable `BytesMut` view.
    BytesMut,
    /// `Object<dyn K>`: an object kind's handle, ownership crossing with
    /// it. Holds the `dyn K` inside.
    Object(Type),
    /// `&mut dyn K` (`K` an object kind): the caller's object, lent for the
    /// call as a pointer to an `ObjectRaw` on its stack; nothing is owed.
    /// Holds the `dyn K`.
    BorrowMut(Type),
    /// `&dyn K`: the same, shared.
    Borrow(Type),
}

/// How a return crosses.
#[derive(Clone)]
enum RetKind {
    Unit,
    Scalar(Type),
    Value,
    Map,
    List,
    /// `String`, as an owned `Text`.
    Text,
    /// Any other type, as a value through `FromValue`.
    Owned(Type),
    /// `Object<dyn K>`, as an `ObjectRaw` the caller then owns. Holds the
    /// `dyn K` inside.
    Object(Type),
}

struct ArgPlan {
    ident: Ident,
    kind: ArgKind,
    ty: Type,
}

struct MethodPlan {
    ident: Ident,
    args: Vec<ArgPlan>,
    ret: RetKind,
    /// The declared return type, verbatim, for the proxy's signature.
    ret_ty: ReturnType,
    fallible: bool,
    /// `&mut self` (object kinds only).
    mutable: bool,
    default: Option<Block>,
    /// The normalised signature the hash covers.
    signature: String,
}

fn plan_methods(tr: &ItemTrait, object: bool, roots: Roots<'_>) -> syn::Result<Vec<MethodPlan>> {
    let mut plans = Vec::new();
    let mut seen_default = false;
    for item in &tr.items {
        let f = match item {
            TraitItem::Fn(f) => f,
            TraitItem::Const(c) => {
                return Err(syn::Error::new_spanned(
                    c,
                    "a kind cannot declare an associated const; a table holds functions only",
                ));
            }
            TraitItem::Type(t) => {
                return Err(syn::Error::new_spanned(
                    t,
                    "a kind cannot declare an associated type; a table holds functions only",
                ));
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "a kind may contain only methods",
                ));
            }
        };
        let plan = plan_method(f, object, roots)?;
        if plan.default.is_some() {
            seen_default = true;
        } else if seen_default {
            return Err(syn::Error::new_spanned(
                &f.sig.ident,
                "a required method cannot follow one with a default body: defaulted methods are \
                 appended slots, and the required ones must be one contiguous prefix",
            ));
        }
        plans.push(plan);
    }
    if plans.is_empty() {
        return Err(syn::Error::new_spanned(
            &tr.ident,
            "a kind needs at least one method; an empty table serves nothing",
        ));
    }
    for m in plans.iter().filter(|m| m.answers_tree()) {
        let added = in_ident(&m.ident).to_string();
        if let Some(clash) = plans.iter().find(|p| p.ident.unraw() == added) {
            return Err(syn::Error::new_spanned(
                &clash.ident,
                format!(
                    "the attribute adds `{added}` beside `{}`, which answers a tree, so a method \
                     cannot have that name; give it another",
                    m.ident.unraw()
                ),
            ));
        }
    }
    Ok(plans)
}

impl ArgKind {
    /// `&mut Value`, `&mut Map` or `&mut List`: the caller's tree, lent.
    fn is_lent_tree(&self) -> bool {
        matches!(self, ArgKind::ValueMut | ArgKind::MapMut | ArgKind::ListMut)
    }
}

impl MethodPlan {
    /// Whether the answer is a `Value`, `Map` or `List`: a tree the caller
    /// wants in its own allocator, which the trait's `<method>_in` builds.
    fn answers_tree(&self) -> bool {
        matches!(self.ret, RetKind::Value | RetKind::Map | RetKind::List)
    }

    /// The name `<method>_in`'s allocator takes: `alloc`, unless an
    /// argument already has it.
    fn alloc_param(&self) -> Ident {
        if self.args.iter().any(|a| a.ident.unraw() == "alloc") {
            format_ident!("alloc_")
        } else {
            format_ident!("alloc")
        }
    }
}

/// `<method>_in`: the method with the caller's allocator, which a shim
/// calls for a tree answer.
fn in_ident(method: &Ident) -> Ident {
    format_ident!("{}_in", method)
}

fn plan_method(f: &TraitItemFn, object: bool, roots: Roots<'_>) -> syn::Result<MethodPlan> {
    let sig = &f.sig;
    if let Some(a) = &sig.asyncness {
        return Err(syn::Error::new_spanned(
            a,
            "an `async fn` cannot cross a C boundary; make it blocking",
        ));
    }
    if let Some(u) = &sig.unsafety {
        return Err(syn::Error::new_spanned(
            u,
            "an `unsafe fn` cannot be a kind method; the proxy calls it from safe code",
        ));
    }
    if sig.generics.params.iter().next().is_some() || sig.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &sig.generics,
            "a kind method cannot be generic: a slot is one monomorphised function",
        ));
    }
    if let Some(v) = &sig.variadic {
        return Err(syn::Error::new_spanned(
            v,
            "a kind method cannot be variadic",
        ));
    }
    let reserved = match sig.ident.unraw().to_string().as_str() {
        "header" => Some("the table's first field is its `header`"),
        "destroy" if object => Some("an object kind's table carries its own `destroy` slot"),
        "into_object" if object => Some("the attribute adds `into_object` to an object kind"),
        _ => None,
    };
    if let Some(why) = reserved {
        return Err(syn::Error::new_spanned(
            &sig.ident,
            format!(
                "a kind method cannot be named `{}`: {why}; give the method another name",
                sig.ident
            ),
        ));
    }
    let mutable = match sig.receiver() {
        Some(r) if r.reference.is_some() && r.mutability.is_none() => false,
        Some(r) if r.reference.is_some() && object => true,
        Some(r) => {
            return Err(syn::Error::new_spanned(
                r,
                "a kind method takes `&self`: a provider is shared between callers, so `&mut \
                 self` and `self` cannot cross; only an object kind (`#[kind(object)]`, a handle \
                 one caller owns) may take `&mut self`",
            ));
        }
        None => {
            return Err(syn::Error::new_spanned(
                &sig.ident,
                "a kind method takes `&self`; an associated function has no provider to call",
            ));
        }
    };

    let mut args = Vec::new();
    for input in sig.inputs.iter().skip(1) {
        let FnArg::Typed(pat) = input else {
            continue;
        };
        let ident = match &*pat.pat {
            syn::Pat::Ident(p) => p.ident.clone(),
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "a kind method's arguments are named, not destructured",
                ));
            }
        };
        if ident.to_string().starts_with("__") {
            return Err(syn::Error::new_spanned(
                &ident,
                "a kind method's argument cannot start with `__`: those names are the generated \
                 glue's",
            ));
        }
        let kind = arg_kind(&pat.ty, object, roots)?;
        args.push(ArgPlan {
            ident,
            kind,
            ty: (*pat.ty).clone(),
        });
    }

    let (ret, fallible) = ret_kind(&sig.output, roots)?;
    let converts = args.iter().any(|a| {
        matches!(
            a.kind,
            ArgKind::Owned(_) | ArgKind::Object(_) | ArgKind::BorrowMut(_) | ArgKind::Borrow(_)
        )
    }) || matches!(ret, RetKind::Owned(_) | RetKind::Object(_));
    if converts && !fallible {
        return Err(syn::Error::new_spanned(
            &sig.ident,
            "a method converting an argument or its return through `ToValue`/`FromValue`, or \
             passing an `Object`, must return `Result<_, ProviderError>`, because the conversion \
             or the validation can fail; so must one lending an object (`&mut dyn K`, `&dyn K`)",
        ));
    }

    // NOT refused here: an infallible method taking `&str`, `&Value` or
    // `&Map`. It looks like the reachable case -- the shim checks each of
    // those where it arrives, and an infallible method has nowhere to put
    // the refusal, so the proxy panics in the HOST. Measured 2026-09-25,
    // it cannot fire: the proxy is the only caller that a panic can reach
    // back to, and what it sends is already checked. `Str::new(&str)` is
    // UTF-8 by type; a `&Value`/`&Map` pointer comes from a live
    // reference, so never null; and a `&Map` a Rust host holds arrived
    // through `checked_arm!(Map, ..)`, whose keys are UTF-8. When the
    // caller is C instead, it reads the status the shim returns and there
    // is no proxy to panic.
    //
    // What IS reachable is a provider -- a C one, or a hand-written table
    // -- answering non-OK for an infallible slot for its own reasons, and
    // no signature rule can prevent that. It is documented in the kind
    // section of the shipped `AGENTS.md` instead, and pinned by
    // `tests/kind_glue_probes.rs`.

    let signature = normalised_signature(sig, roots);
    Ok(MethodPlan {
        ident: sig.ident.clone(),
        args,
        ret,
        ret_ty: sig.output.clone(),
        fallible,
        mutable,
        default: f.default.clone(),
        signature,
    })
}

/// The `dyn K` inside an `Object<dyn K>` path, `None` for any other
/// type, or a refusal for `other::Object<dyn K>`: that shape is only ever
/// guatiao's handle, which the attribute cannot tell it is.
fn object_inner(ty: &Type, roots: Roots<'_>) -> syn::Result<Option<Type>> {
    let Type::Path(p) = ty else {
        return Ok(None);
    };
    if p.qself.is_some() {
        return Ok(None);
    }
    let last = p.path.segments.last().expect("a path has a segment");
    let PathArguments::AngleBracketed(a) = &last.arguments else {
        return Ok(None);
    };
    let Some(GenericArgument::Type(inner)) = a.args.first() else {
        return Ok(None);
    };
    if roots.guatiao(&p.path) == Some("Object") {
        return Ok(Some(inner.clone()));
    }
    if roots.unrecognised(&p.path) == Some("Object") && matches!(inner, Type::TraitObject(_)) {
        return Err(roots.refusal(ty, "Object"));
    }
    Ok(None)
}

/// Which of the types the glue treats specially `ty` names — guatiao's
/// `Value`, `Map`, `List`, `Text`, `ProviderError`, `Object`, `Status`
/// or the standard `String`, `Vec`, `Option`, `Result`, `Box` — or
/// `None`. Used where a type is borrowed, returned or named as the error:
/// there a path ending in `Value`, `Map`, `List`, `String`,
/// `ProviderError` or `Object` under a root the attribute does not read
/// can only be meant as guatiao's or the standard one, so it is refused
/// rather than misread. An argument by value reads [`special_by_value`].
fn special(ty: &Type, roots: Roots<'_>) -> syn::Result<Option<&'static str>> {
    let name = special_by_value(ty, roots);
    if name.is_some() {
        return Ok(name);
    }
    let Type::Path(p) = ty else {
        return Ok(None);
    };
    if p.qself.is_some() {
        return Ok(None);
    }
    match roots.unrecognised(&p.path) {
        Some(name @ ("Value" | "Map" | "List" | "String" | "ProviderError" | "Object")) => {
            Err(roots.refusal(ty, name))
        }
        _ => Ok(None),
    }
}

/// [`special`] for an argument taken by value: a path the attribute does
/// not read is an ordinary type of the caller's whatever its last
/// segment, crossing through `ToValue`/`FromValue` (`proto::String`).
fn special_by_value(ty: &Type, roots: Roots<'_>) -> Option<&'static str> {
    let Type::Path(p) = ty else {
        return None;
    };
    if p.qself.is_some() {
        return None;
    }
    roots.guatiao(&p.path).or_else(|| roots.std(&p.path))
}

/// The last path segment's ident, for a plain path type.
fn last_ident(ty: &Type) -> Option<&Ident> {
    match ty {
        Type::Path(p) if p.qself.is_none() => p.path.segments.last().map(|s| &s.ident),
        _ => None,
    }
}

fn is_scalar(ident: &Ident) -> bool {
    matches!(
        ident.to_string().as_str(),
        "bool"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
    )
}

/// `&mut dyn K` or `&dyn K`: whether it is mutable, and the `dyn K`.
/// `None` for a reference to anything else.
fn lent_inner(r: &TypeReference) -> Option<syn::Result<(bool, Type)>> {
    let mut elem = &*r.elem;
    while let Type::Paren(syn::TypeParen { elem: inner, .. })
    | Type::Group(syn::TypeGroup { elem: inner, .. }) = elem
    {
        elem = inner;
    }
    let Type::TraitObject(t) = elem else {
        return None;
    };
    let mut bounds = t.bounds.iter();
    let (Some(syn::TypeParamBound::Trait(bound)), None) = (bounds.next(), bounds.next()) else {
        return Some(Err(syn::Error::new_spanned(
            elem,
            "lend an object as `&mut dyn K` or `&dyn K`, naming the kind's trait alone: the \
             table is the kind's, and another bound or a lifetime has none",
        )));
    };
    let path = &bound.path;
    Some(Ok((r.mutability.is_some(), syn::parse_quote!(dyn #path))))
}

fn arg_kind(ty: &Type, object: bool, roots: Roots<'_>) -> syn::Result<ArgKind> {
    if let Some(inner) = object_inner(ty, roots)? {
        return Ok(ArgKind::Object(inner));
    }
    if let Type::Reference(r) = ty
        && let Some(lent) = lent_inner(r)
    {
        let (mutable, inner) = lent?;
        return Ok(if mutable {
            ArgKind::BorrowMut(inner)
        } else {
            ArgKind::Borrow(inner)
        });
    }
    match ty {
        Type::Reference(TypeReference {
            mutability: Some(_),
            elem,
            ..
        }) if object
            && matches!(&**elem, Type::Slice(s) if last_ident(&s.elem).is_some_and(|i| i == "u8")) =>
        {
            Ok(ArgKind::BytesMut)
        }
        Type::Reference(TypeReference {
            mutability: Some(_),
            elem,
            ..
        }) if matches!(&**elem, Type::Path(_))
            && matches!(special(elem, roots)?, Some("Value" | "Map" | "List")) =>
        {
            Ok(match special(elem, roots)? {
                Some("Value") => ArgKind::ValueMut,
                Some("Map") => ArgKind::MapMut,
                _ => ArgKind::ListMut,
            })
        }
        Type::Reference(TypeReference {
            mutability: Some(m),
            ..
        }) => Err(syn::Error::new_spanned(
            m,
            "a `&mut` argument of this type cannot cross: the callee would write into memory the \
             caller still describes. `&mut Map`, `&mut List` and `&mut Value` are lent for the \
             call, as trees the callee may change and grow; and `&mut [u8]` crosses as an \
             out-buffer, on an object kind only",
        )),
        Type::Reference(TypeReference { elem, .. }) => match &**elem {
            Type::Path(_) if last_ident(elem).is_some_and(|i| i == "str") => Ok(ArgKind::Str),
            Type::Slice(s) if last_ident(&s.elem).is_some_and(|i| i == "u8") => Ok(ArgKind::Bytes),
            Type::Path(_) if special(elem, roots)? == Some("Value") => Ok(ArgKind::ValueRef),
            Type::Path(_) if special(elem, roots)? == Some("Map") => Ok(ArgKind::MapRef),
            _ => Err(syn::Error::new_spanned(
                ty,
                "a borrowed argument of this type cannot cross; `&str`, `&[u8]`, `&Value` and \
                 `&Map` can, or take it by value and it crosses through `ToValue`",
            )),
        },
        Type::ImplTrait(_) => Err(syn::Error::new_spanned(
            ty,
            "an `impl Trait` argument cannot cross a C boundary",
        )),
        Type::BareFn(_) => Err(syn::Error::new_spanned(
            ty,
            "a function argument cannot cross; a callback is a kind of its own",
        )),
        Type::Path(p) => {
            let last = p.path.segments.last().expect("a path has a segment");
            if is_scalar(&last.ident) && last.arguments.is_none() {
                return Ok(ArgKind::Scalar(ty.clone()));
            }
            if special_by_value(ty, roots) == Some("Option")
                && let PathArguments::AngleBracketed(a) = &last.arguments
                && let Some(GenericArgument::Type(Type::Reference(r))) = a.args.first()
                && r.mutability.is_some()
            {
                return Err(syn::Error::new_spanned(
                    ty,
                    "an `Option<&mut T>` argument cannot cross: lend `&mut Value`, which can hold \
                     null, or an empty `&mut Map` or `&mut List`",
                ));
            }
            if special_by_value(ty, roots) == Some("Option")
                && let PathArguments::AngleBracketed(a) = &last.arguments
                && let Some(GenericArgument::Type(Type::Reference(r))) = a.args.first()
                && special(&r.elem, roots)? == Some("Value")
            {
                return Ok(ArgKind::ValueOpt);
            }
            match special_by_value(ty, roots) {
                Some("Value") => return Ok(ArgKind::ValueOwned),
                Some("Map") => return Ok(ArgKind::MapOwned),
                Some("List") => return Ok(ArgKind::ListOwned),
                Some("Text") => {
                    return Err(syn::Error::new_spanned(
                        ty,
                        "a `Text` cannot cross by value; take `&str`, or a `Value` holding it",
                    ));
                }
                _ if last.ident == "Buffer" => {
                    return Err(syn::Error::new_spanned(
                        ty,
                        "a `Buffer` cannot cross by value; take `&[u8]`, or a `Value` holding it",
                    ));
                }
                _ => {}
            }
            Ok(ArgKind::Owned(ty.clone()))
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "an argument of this shape cannot cross a C boundary",
        )),
    }
}

fn ret_kind(output: &ReturnType, roots: Roots<'_>) -> syn::Result<(RetKind, bool)> {
    let ty = match output {
        ReturnType::Default => return Ok((RetKind::Unit, false)),
        ReturnType::Type(_, ty) => &**ty,
    };
    // `Result<X, ProviderError>` unwraps to X, fallible.
    if let Type::Path(p) = ty
        && let Some(last) = p.path.segments.last()
        && last.ident == "Result"
    {
        let PathArguments::AngleBracketed(a) = &last.arguments else {
            return Err(syn::Error::new_spanned(
                ty,
                "spell the error type: `Result<_, ProviderError>`",
            ));
        };
        let mut it = a.args.iter();
        let (Some(GenericArgument::Type(ok)), Some(GenericArgument::Type(err))) =
            (it.next(), it.next())
        else {
            return Err(syn::Error::new_spanned(
                ty,
                "spell the error type: `Result<_, ProviderError>`",
            ));
        };
        if special(err, roots)? != Some("ProviderError") {
            return Err(syn::Error::new_spanned(
                err,
                "the error of a kind method is `ProviderError`, the one error type both sides \
                 of the table share",
            ));
        }
        return Ok((plain_ret(ok, roots)?, true));
    }
    Ok((plain_ret(ty, roots)?, false))
}

fn plain_ret(ty: &Type, roots: Roots<'_>) -> syn::Result<RetKind> {
    if let Some(inner) = object_inner(ty, roots)? {
        return Ok(RetKind::Object(inner));
    }
    match ty {
        Type::Tuple(t) if t.elems.is_empty() => Ok(RetKind::Unit),
        Type::Reference(_) => Err(syn::Error::new_spanned(
            ty,
            "a borrowed return cannot cross: nothing on this side owns what it would borrow. \
             Return it by value",
        )),
        Type::ImplTrait(_) => Err(syn::Error::new_spanned(
            ty,
            "an `impl Trait` return cannot cross a C boundary",
        )),
        Type::Path(p) => {
            let last = p.path.segments.last().expect("a path has a segment");
            if is_scalar(&last.ident) && last.arguments.is_none() {
                return Ok(RetKind::Scalar(ty.clone()));
            }
            Ok(match special(ty, roots)? {
                Some("Value") => RetKind::Value,
                Some("Map") => RetKind::Map,
                Some("List") => RetKind::List,
                Some("String") => RetKind::Text,
                _ => RetKind::Owned(ty.clone()),
            })
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "a return of this shape cannot cross a C boundary",
        )),
    }
}

/// `name(ty,ty)->ret`, each type in its canonical spelling with every
/// space removed: a re-typed or reordered required slot changes the hash,
/// and a reformatted one, or one naming a standard or `guatiao` type by
/// another path, does not. See [`Roots::canonical`] for what is
/// recognised; any other spelling is hashed as written.
fn normalised_signature(sig: &syn::Signature, roots: Roots<'_>) -> String {
    let spelled = |ty: &Type| {
        let ty = roots.canonical(ty);
        strip(&quote!(#ty))
    };
    let args: Vec<String> = sig
        .inputs
        .iter()
        .skip(1)
        .filter_map(|a| match a {
            FnArg::Typed(t) => Some(spelled(&t.ty)),
            FnArg::Receiver(_) => None,
        })
        .collect();
    let ret = match &sig.output {
        ReturnType::Default => "()".to_string(),
        ReturnType::Type(_, ty) => spelled(ty),
    };
    format!("{}({})->{}", sig.ident, args.join(","), ret)
}

fn strip(tokens: &TokenStream) -> String {
    tokens
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

// --- the expansion ---------------------------------------------------------

fn emit(tr: &ItemTrait, attr: &KindAttr, methods: &[MethodPlan]) -> TokenStream {
    let trait_ident = &tr.ident;
    let vis = &tr.vis;
    let name = &attr.name;
    let object = attr.object;
    let table = format_ident!("{}Vtable", trait_ident);

    let required: Vec<&MethodPlan> = methods.iter().filter(|m| m.default.is_none()).collect();
    // An object kind's hash names the shape, so a provider table and an
    // object table with the same methods can never pass for each other.
    let hash_input = core::iter::once(if object { "object" } else { "provider" }.to_string())
        .chain(required.iter().map(|m| m.signature.clone()))
        .collect::<Vec<_>>()
        .join(";");
    // Computed HERE and written as a literal, not left to the runtime's
    // `fnv1a` at compile time: a C header generator renders an
    // associated constant only when its value is a literal, and the hash
    // is what a C implementation writes into its table's header.
    let floor_hash = fnv1a(&hash_input);
    let floor_hash = syn::LitInt::new(&floor_hash.to_string(), proc_macro2::Span::call_site());
    // The floor: one past the last required slot, or past `destroy` for
    // an object kind whose methods all have defaults.
    let floor_end = match required.last() {
        Some(last) => end_ident(&last.ident),
        None => format_ident!("destroy_end"),
    };

    // Counts the objects this image made and has not seen destroyed.
    let live = format_ident!("__GUATIAO_LIVE_{}", trait_ident.to_string().to_uppercase());
    // An object kind's table starts with `destroy`, right after the header.
    let destroy_field = object.then(|| {
        quote! { pub destroy: ::core::option::Option<unsafe extern "C" fn(ctx: *mut ::core::ffi::c_void)>, }
    });
    let destroy_init = object.then(|| {
        quote! { destroy: ::core::option::Option::Some(Self::__guatiao_destroy::<__T>), }
    });
    let destroy_items = object.then(|| {
        quote! {
            #[doc(hidden)]
            pub const fn destroy_end() -> usize {
                ::core::mem::offset_of!(Self, destroy) + ::core::mem::size_of::<usize>()
            }

            /// # Safety
            ///
            /// `ctx` is null or the cell `into_object` boxed for a `__T`,
            /// not used again.
            #[doc(hidden)]
            pub unsafe extern "C" fn __guatiao_destroy<__T: #trait_ident>(ctx: *mut ::core::ffi::c_void) {
                ::guatiao::library::kind::catch(|| {
                    if !ctx.is_null() {
                        #live.fetch_sub(1, ::core::sync::atomic::Ordering::Relaxed);
                    }
                    // SAFETY: the caller's contract.
                    unsafe { ::guatiao::library::kind::destroy_object::<Self, __T>(ctx) };
                    ::guatiao::Status::GUATIAO_OK
                });
            }
        }
    });
    let destroy_required = object.then(|| quote! { ("destroy", #table::destroy_end()), });

    // A slot lending a tree says which argument, so a C reader can tell it
    // from one handed over: both are a pointer without `const`.
    let slot_fields = methods.iter().map(|m| {
        let ident = &m.ident;
        let ty = slot_type(m);
        let lent: Vec<String> = m
            .args
            .iter()
            .filter(|a| a.kind.is_lent_tree())
            .map(|a| format!("`{}`", c_param(&a.ident)))
            .collect();
        let doc = (!lent.is_empty()).then(|| {
            let line = format!(" Lent for the call, not handed over: {}.", lent.join(", "));
            quote! { #[doc = #line] }
        });
        quote! {
            #doc
            pub #ident: ::core::option::Option<#ty>,
        }
    });
    let slot_inits = methods.iter().map(|m| {
        let ident = &m.ident;
        let shim = shim_ident(&m.ident);
        quote! { #ident: ::core::option::Option::Some(Self::#shim::<__T>), }
    });
    let end_fns = methods.iter().map(|m| {
        let ident = &m.ident;
        let end = end_ident(&m.ident);
        quote! {
            #[doc(hidden)]
            pub const fn #end() -> usize {
                ::core::mem::offset_of!(Self, #ident) + ::core::mem::size_of::<usize>()
            }
        }
    });
    let receiver = if object {
        Receiver::Cell
    } else {
        Receiver::Provider
    };
    let shims = methods
        .iter()
        .map(|m| emit_shim(trait_ident, &table, receiver, m));
    let required_slots = required.iter().map(|m| {
        let name = m.ident.to_string();
        let end = end_ident(&m.ident);
        quote! { (#name, #table::#end()), }
    });
    let proxies = methods.iter().map(|m| emit_proxy(&table, m));

    // An object kind's lent tables: one whose slots reach the lender's
    // `*mut dyn Trait`, and, when the kind has a `&self` method, one over
    // a `*const dyn Trait` whose `&mut self` slots refuse. `destroy` does
    // nothing in either: a lent object stays its lender's.
    let shared_methods = methods.iter().any(|m| !m.mutable);
    let lent_items = object.then(|| {
        let lent_mut_shims = methods
            .iter()
            .map(|m| emit_shim(trait_ident, &table, Receiver::LentMut, m));
        let lent_mut_inits = methods.iter().map(|m| {
            let ident = &m.ident;
            let shim = lent_mut_shim_ident(&m.ident);
            quote! { #ident: ::core::option::Option::Some(Self::#shim), }
        });
        let shared = shared_methods.then(|| {
            let lent_shims = methods
                .iter()
                .map(|m| emit_shim(trait_ident, &table, Receiver::Lent, m));
            let lent_inits = methods.iter().map(|m| {
                let ident = &m.ident;
                let shim = lent_shim_ident(&m.ident);
                quote! { #ident: ::core::option::Option::Some(Self::#shim), }
            });
            quote! {
                /// The table a value is lent through as `&dyn` for one call.
                #[doc(hidden)]
                pub const fn lent() -> Self {
                    Self {
                        header: ::guatiao::library::KindHeader::new(
                            ::core::mem::size_of::<Self>(),
                            <dyn #trait_ident as ::guatiao::library::Kind>::FLOOR_HASH,
                        ),
                        destroy: ::core::option::Option::Some(Self::__guatiao_lent_destroy),
                        #(#lent_inits)*
                    }
                }

                #(#lent_shims)*
            }
        });
        quote! {
            /// The table a value is lent through as `&mut dyn` for one call.
            #[doc(hidden)]
            pub const fn lent_mut() -> Self {
                Self {
                    header: ::guatiao::library::KindHeader::new(
                        ::core::mem::size_of::<Self>(),
                        <dyn #trait_ident as ::guatiao::library::Kind>::FLOOR_HASH,
                    ),
                    destroy: ::core::option::Option::Some(Self::__guatiao_lent_destroy),
                    #(#lent_mut_inits)*
                }
            }

            /// # Safety
            ///
            /// None needed: a lent object is its lender's, so this frees
            /// nothing, whoever calls it.
            #[doc(hidden)]
            pub unsafe extern "C" fn __guatiao_lent_destroy(_ctx: *mut ::core::ffi::c_void) {}

            #(#lent_mut_shims)*

            #shared
        }
    });
    // Every kind is `Lend`, so a lent argument of a provider kind meets the
    // sentence below rather than a missing impl; only an object kind's
    // carries tables.
    let lent_consts = object.then(|| {
        let shared = shared_methods.then(|| {
            quote! {
                const LENT: ::core::option::Option<&'static #table> = {
                    const __LENT: #table = #table::lent();
                    ::core::option::Option::Some(&__LENT)
                };
            }
        });
        quote! {
            const LENT_MUT: ::core::option::Option<&'static #table> = {
                const __LENT: #table = #table::lent_mut();
                ::core::option::Option::Some(&__LENT)
            };
            #shared
        }
    });

    // A lent argument names an object kind that can be lent that way,
    // checked where the kind's impl is visible: at compile time, with a
    // sentence, rather than as a null table refused at run time.
    let lent_checks = methods
        .iter()
        .flat_map(|m| &m.args)
        .filter_map(|a| match &a.kind {
            ArgKind::BorrowMut(inner) => Some(quote! {
                const _: () = ::core::assert!(
                    <#inner as ::guatiao::library::kind::Lend>::LENT_MUT.is_some(),
                    "a `&mut dyn K` argument lends an object kind (`#[guatiao::kind(object)]`); a \
                     provider kind is shared and offered, never lent",
                );
            }),
            ArgKind::Borrow(inner) => Some(quote! {
                const _: () = ::core::assert!(
                    <#inner as ::guatiao::library::kind::Lend>::LENT.is_some(),
                    "a `&dyn K` argument lends an object kind with a `&self` method; this one has \
                     none to call, or is a provider kind: take `&mut dyn K`, or give the kind one",
                );
            }),
            _ => None,
        });
    // A provider kind's instance is the trait as well, so a host stores it
    // as `Arc<dyn Trait>` or `Box<dyn Trait>` and it is released with the
    // last owner. An object kind has no instances.
    let instance_impl = (!object).then(|| {
        let forwards = methods.iter().map(emit_forward);
        quote! {
            impl #trait_ident for ::guatiao::library::Instance<dyn #trait_ident> {
                #(#forwards)*
            }

            impl ::core::convert::From<::guatiao::library::Instance<dyn #trait_ident>>
                for ::guatiao::__private::alloc::boxed::Box<dyn #trait_ident>
            {
                fn from(instance: ::guatiao::library::Instance<dyn #trait_ident>) -> Self {
                    ::guatiao::__private::alloc::boxed::Box::new(instance)
                }
            }
        }
    });

    // Objects this image made and has not seen destroyed, and the kinds
    // this one's methods hand back, which a library answers for as well.
    let live_static = object.then(|| {
        quote! {
            #[doc(hidden)]
            static #live: ::core::sync::atomic::AtomicUsize =
                ::core::sync::atomic::AtomicUsize::new(0);
        }
    });
    let own_live = if object {
        quote! { #live.load(::core::sync::atomic::Ordering::Relaxed) }
    } else {
        quote! { 0usize }
    };
    let mut returned: Vec<Type> = Vec::new();
    for m in methods.iter() {
        if let RetKind::Object(inner) = &m.ret {
            let spelled = quote!(#inner).to_string();
            if !returned.iter().any(|t| quote!(#t).to_string() == spelled) {
                returned.push(inner.clone());
            }
        }
    }

    // What pairs an object cell's table with its value by type: the table
    // for `__T` is `of::<__T>()`, and only for a `__T` of this kind.
    let table_for = object.then(|| {
        quote! {
            // SAFETY: `of::<__T>()` is this kind's table for `__T`: its
            // slots read `ctx` as an `ObjectCell<Self, __T>` and its
            // `destroy` frees one.
            unsafe impl<__T: #trait_ident> ::guatiao::library::kind::TableFor<__T> for #table {
                type Kind = dyn #trait_ident;
                fn table() -> Self {
                    Self::of::<__T>()
                }
            }
        }
    });

    // What a C provider of a table with a tree answer needs to know.
    let carrier_doc = methods
        .iter()
        .any(|m| {
            matches!(
                m.ret,
                RetKind::Value | RetKind::Map | RetKind::List | RetKind::Owned(_)
            )
        })
        .then(|| {
            quote! {
                ///
                /// A slot answering a value, a map or a list finds its out
                /// slot holding a carrier, which owns nothing: an empty map
                /// or list, or an ABSENT value, whose `alloc` (the map
                /// arm's, for a value) names the allocator the caller wants
                /// the answer in, or is null. Build the answer through it
                /// and the caller keeps it as it is; built anywhere else,
                /// the caller copies it. Either way, write the answer over
                /// the carrier without freeing it.
            }
        });

    // What a C provider of a table taking a tree by value, or lent, needs
    // to know.
    let lends = methods
        .iter()
        .any(|m| m.args.iter().any(|a| a.kind.is_lent_tree()));
    let moved_doc = methods
        .iter()
        .any(|m| {
            m.args.iter().any(|a| {
                matches!(
                    a.kind,
                    ArgKind::ValueOwned | ArgKind::MapOwned | ArgKind::ListOwned
                )
            })
        })
        .then(|| {
            if lends {
                quote! {
                    ///
                    /// A `guatiao_value *`, `guatiao_map *` or `guatiao_list *`
                    /// argument that is not `const`, and that its slot's comment
                    /// does not call lent, is a tree the caller hands over with
                    /// the call: the slot owns it from then on, on every path,
                    /// and frees or keeps it; the caller never frees it. It lives
                    /// in the caller's allocators.
                }
            } else {
                quote! {
                    ///
                    /// A `guatiao_value *`, `guatiao_map *` or `guatiao_list *`
                    /// argument that is not `const` is a tree the caller hands
                    /// over with the call: the slot owns it from then on, on
                    /// every path, and frees or keeps it; the caller never frees
                    /// it. It lives in the caller's allocators.
                }
            }
        });
    let lent_doc = lends.then(|| {
        quote! {
            ///
            /// A `guatiao_value *`, `guatiao_map *` or `guatiao_list *`
            /// argument its slot's comment calls lent is the caller's tree,
            /// for the call only: read it, change it, grow it, but never free
            /// it or keep the pointer. Build what goes into it through the
            /// allocator it records (`alloc`, the map arm's for a value); a
            /// Rust caller moves anything built elsewhere there once the call
            /// returns.
        }
    });

    // The trait, with `into_object` appended for an object kind: the way
    // an implementation becomes a handle. `where Self: Sized` keeps it off
    // the trait object, so it is not a slot.
    let mut tr = tr.clone();
    if object {
        let into_object: TraitItem = syn::parse_quote! {
            /// This implementation as a handle: boxed beside its table,
            /// destroyed when the handle is dropped.
            fn into_object(self) -> ::guatiao::library::Object<dyn #trait_ident>
            where
                Self: Sized + Send + 'static,
            {
                let cell = ::guatiao::__private::alloc::boxed::Box::new(
                    ::guatiao::library::kind::ObjectCell::<#table, Self>::new(self),
                );
                let object = ::guatiao::library::Object::<dyn #trait_ident>::from_cell(cell)
                    .expect("a table the kind attribute built fits its own kind");
                #live.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
                object
            }
        };
        tr.items.push(into_object);
    }
    // And `<method>_in` beside every method answering a tree: what a shim
    // calls, naming the caller's allocator. Provided, so an implementation
    // that does not build there still compiles and is copied.
    for m in methods.iter().filter(|m| m.answers_tree()) {
        let method = &m.ident;
        let in_method = in_ident(method);
        let alloc = m.alloc_param();
        let receiver = if m.mutable {
            quote!(&mut self)
        } else {
            quote!(&self)
        };
        let params = m.args.iter().map(|a| {
            let ident = &a.ident;
            let ty = &a.ty;
            quote!(#ident: #ty)
        });
        let names = m.args.iter().map(|a| &a.ident);
        let ret_ty = &m.ret_ty;
        let summary = format!(" `{}`, its answer built through `{alloc}`.", method.unraw());
        let item: TraitItem = syn::parse_quote! {
            #[doc = #summary]
            ///
            /// What a call through the kind's table runs, naming the
            /// allocator the caller wants the answer in. Build the answer
            /// there, with the `_in` forms, and it crosses without a copy.
            /// The default answers the method itself, which the caller
            /// then copies into `alloc`.
            #[allow(unused_variables)]
            fn #in_method(#receiver #(, #params)*, #alloc: ::guatiao::Alloc) #ret_ty {
                self.#method(#(#names),*)
            }
        };
        tr.items.push(item);
    }

    quote! {
        #live_static

        #tr

        /// The kind's function table: a header, then one slot per method,
        /// in declaration order.
        #carrier_doc
        #moved_doc
        #lent_doc
        #[repr(C)]
        #[doc(hidden)]
        #[allow(non_snake_case)]
        #vis struct #table {
            pub header: ::guatiao::library::KindHeader,
            #destroy_field
            #(#slot_fields)*
        }

        #[allow(non_snake_case, clippy::missing_safety_doc)]
        impl #table {
            /// The table for one implementation of the kind.
            #[doc(hidden)]
            pub const fn of<__T: #trait_ident>() -> Self {
                Self {
                    header: ::guatiao::library::KindHeader::new(
                        ::core::mem::size_of::<Self>(),
                        <dyn #trait_ident as ::guatiao::library::Kind>::FLOOR_HASH,
                    ),
                    #destroy_init
                    #(#slot_inits)*
                }
            }

            /// One past the last required slot. Frozen.
            #[doc(hidden)]
            pub const fn floor() -> usize {
                Self::#floor_end()
            }

            /// FNV-1a over the shape and the required signatures, as a
            /// literal, so a C header generator can render it: what a C
            /// implementation writes into the table's header.
            pub const FLOOR_HASH: u32 = #floor_hash;

            #destroy_items

            #(#end_fns)*

            #(#shims)*

            #lent_items
        }

        #(#lent_checks)*

        impl ::guatiao::library::Kind for dyn #trait_ident {
            const NAME: &'static str = #name;
            type Vtable = #table;
            const FLOOR: usize = #table::floor();
            const FLOOR_HASH: u32 = #table::FLOOR_HASH;
            const REQUIRED: &'static [(&'static str, usize)] = &[ #destroy_required #(#required_slots)* ];
            const OBJECT: bool = #object;

            fn live_objects(seen: &mut ::guatiao::__private::alloc::vec::Vec<&'static str>) -> usize {
                if seen.contains(&#name) {
                    return 0;
                }
                seen.push(#name);
                #own_live #(+ <#returned as ::guatiao::library::Kind>::live_objects(seen))*
            }

            fn as_dyn(remote: &::guatiao::library::Remote<Self>) -> &Self {
                remote
            }
            fn as_dyn_mut(remote: &mut ::guatiao::library::Remote<Self>) -> &mut Self {
                remote
            }
            fn boxed(remote: ::guatiao::library::Remote<Self>) -> ::guatiao::__private::alloc::boxed::Box<Self> {
                ::guatiao::__private::alloc::boxed::Box::new(remote)
            }
            fn shared(remote: ::guatiao::library::Remote<Self>) -> ::guatiao::__private::alloc::sync::Arc<Self> {
                ::guatiao::__private::alloc::sync::Arc::new(remote)
            }
        }

        // SAFETY: the tables are this kind's `lent_mut()` and `lent()`,
        // whose slots read `ctx` as the lender's pointer, shared for a
        // `&self` method, and whose `destroy` frees nothing.
        unsafe impl ::guatiao::library::kind::Lend for dyn #trait_ident {
            #lent_consts
        }

        #table_for

        #[allow(clippy::needless_question_mark, clippy::redundant_closure_call)]
        impl #trait_ident for ::guatiao::library::Remote<dyn #trait_ident> {
            #(#proxies)*
        }

        impl ::core::convert::From<::guatiao::library::Remote<dyn #trait_ident>>
            for ::guatiao::__private::alloc::boxed::Box<dyn #trait_ident>
        {
            fn from(remote: ::guatiao::library::Remote<dyn #trait_ident>) -> Self {
                ::guatiao::__private::alloc::boxed::Box::new(remote)
            }
        }

        #instance_impl
    }
}

fn end_ident(method: &Ident) -> Ident {
    format_ident!("{}_end", method)
}

/// FNV-1a, 32-bit: the same function as `guatiao::library::kind::fnv1a`,
/// which a proc macro cannot call. The two agree by test, on both sides.
pub(crate) fn fnv1a(s: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in s.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn shim_ident(method: &Ident) -> Ident {
    format_ident!("__guatiao_shim_{}", method)
}

/// The `&mut` lent table's shim. Its prefix and [`lent_shim_ident`]'s
/// differ before either ends, so no two methods' shims share a name, `x`
/// and `mut_x` included.
fn lent_mut_shim_ident(method: &Ident) -> Ident {
    format_ident!("__guatiao_lentmut__{}", method)
}

/// The shared lent table's shim.
fn lent_shim_ident(method: &Ident) -> Ident {
    format_ident!("__guatiao_lent__{}", method)
}

/// The C type of one argument.
fn arg_c_type(kind: &ArgKind) -> TokenStream {
    match kind {
        ArgKind::Scalar(ty) => quote!(#ty),
        ArgKind::Str => quote!(::guatiao::Str),
        ArgKind::Bytes => quote!(::guatiao::value::types::Bytes),
        ArgKind::ValueRef | ArgKind::ValueOpt | ArgKind::Owned(_) => {
            quote!(*const ::guatiao::Value)
        }
        ArgKind::MapRef => quote!(*const ::guatiao::Map),
        ArgKind::ValueOwned | ArgKind::ValueMut => quote!(*mut ::guatiao::Value),
        ArgKind::MapOwned | ArgKind::MapMut => quote!(*mut ::guatiao::Map),
        ArgKind::ListOwned | ArgKind::ListMut => quote!(*mut ::guatiao::List),
        ArgKind::BytesMut => quote!(::guatiao::library::BytesMut),
        ArgKind::Object(_) => quote!(::guatiao::library::ObjectRaw),
        ArgKind::BorrowMut(_) | ArgKind::Borrow(_) => {
            quote!(*const ::guatiao::library::ObjectRaw)
        }
    }
}

/// The C type of the out-pointer a return crosses through, or `None`
/// for `()`.
fn ret_c_type(ret: &RetKind) -> Option<TokenStream> {
    match ret {
        RetKind::Unit => None,
        RetKind::Scalar(ty) => Some(quote!(*mut #ty)),
        RetKind::Value | RetKind::Owned(_) => Some(quote!(*mut ::guatiao::Value)),
        RetKind::Map => Some(quote!(*mut ::guatiao::Map)),
        RetKind::List => Some(quote!(*mut ::guatiao::List)),
        RetKind::Text => Some(quote!(*mut ::guatiao::Text)),
        RetKind::Object(_) => Some(quote!(*mut ::guatiao::library::ObjectRaw)),
    }
}

/// An argument's name in the slot's type, which a C header renders: one
/// named `ctx`, `out` or `err` gains a `_`, so no two parameters of the
/// prototype share a name.
fn c_param(ident: &Ident) -> Ident {
    if matches!(ident.to_string().as_str(), "ctx" | "out" | "err") {
        format_ident!("{}_", ident)
    } else {
        ident.clone()
    }
}

/// The slot's function-pointer type.
fn slot_type(m: &MethodPlan) -> TokenStream {
    let args = m.args.iter().map(|a| {
        let ident = c_param(&a.ident);
        let ty = arg_c_type(&a.kind);
        quote!(#ident: #ty)
    });
    let out = ret_c_type(&m.ret).map(|ty| quote!(, out: #ty));
    let err = m
        .fallible
        .then(|| quote!(, err: *mut ::guatiao::library::ProviderError));
    quote! {
        unsafe extern "C" fn(ctx: *mut ::core::ffi::c_void #(, #args)* #out #err) -> ::guatiao::Status
    }
}

/// What a shim's `ctx` is, and so how it reaches the implementation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Receiver {
    /// A provider's `&T`; the shim is generic over `T`.
    Provider,
    /// The `ObjectCell` an object was boxed in; generic over `T`.
    Cell,
    /// The address of a lender's `*mut dyn Trait`, for one call.
    LentMut,
    /// The address of a lender's `*const dyn Trait`, for one call: a
    /// `&mut self` method refuses.
    Lent,
}

/// The shim: reads its arguments, calls the implementation, writes the
/// answer. Every `unsafe` is a call into the runtime.
fn emit_shim(
    trait_ident: &Ident,
    table: &Ident,
    receiver: Receiver,
    m: &MethodPlan,
) -> TokenStream {
    let shim = match receiver {
        Receiver::Provider | Receiver::Cell => shim_ident(&m.ident),
        Receiver::LentMut => lent_mut_shim_ident(&m.ident),
        Receiver::Lent => lent_shim_ident(&m.ident),
    };
    let generics = match receiver {
        Receiver::Provider | Receiver::Cell => quote!(<__T: #trait_ident>),
        Receiver::LentMut | Receiver::Lent => quote!(),
    };
    let method = &m.ident;
    let params = m.args.iter().map(|a| {
        let ident = &a.ident;
        let ty = arg_c_type(&a.kind);
        quote!(#ident: #ty)
    });
    let out_param = ret_c_type(&m.ret).map(|ty| quote!(, __out: #ty));
    let err_param = m
        .fallible
        .then(|| quote!(, __err: *mut ::guatiao::library::ProviderError));

    // A conversion failure inside a fallible method is an error the
    // caller sees; the plan refuses conversions in infallible ones.
    let fail = |status: TokenStream| {
        if m.fallible {
            quote! {
                // SAFETY: the proxy passes a writable error slot.
                return unsafe { ::guatiao::library::kind::write_err(__err, #status.into()) };
            }
        } else {
            quote! { return #status; }
        }
    };

    // An object argument is OWNED the moment the call is made: whatever
    // happens next, this side must destroy it. So objects are taken first,
    // before `ctx` is checked and before any other argument can fail —
    // once bound, an early `return` drops the handle and that runs the
    // destroy. A provider with no instance (`ctx` null) refusing a call
    // must not leak what it was handed.
    let object_reads = m.args.iter().filter_map(|a| {
        let ident = &a.ident;
        let ArgKind::Object(inner) = &a.kind else {
            return None;
        };
        // Always fallible: the plan requires `Result` for an object.
        Some(quote! {
            // SAFETY: the proxy passes an object it handed across.
            let #ident = match unsafe { ::guatiao::library::kind::object_arg::<#inner>(#ident) } {
                ::core::result::Result::Ok(__v) => __v,
                // SAFETY: the proxy passes a writable error slot.
                ::core::result::Result::Err(__e) => {
                    return unsafe { ::guatiao::library::kind::write_err(__err, __e) };
                }
            };
        })
    });

    // A tree by value is owned from the call on, so it is taken before
    // anything can return early: held in a `Result`, it is freed by any
    // `return` that follows, and only a null pointer, which owns nothing,
    // or a map whose keys are refused (freed on the spot) is an `Err`.
    let tree_takes: Vec<TokenStream> = m
        .args
        .iter()
        .filter_map(|a| {
            let ident = &a.ident;
            let take = match &a.kind {
                ArgKind::ValueOwned => quote!(value_owned),
                ArgKind::MapOwned => quote!(map_owned),
                ArgKind::ListOwned => quote!(list_owned),
                _ => return None,
            };
            Some(quote! {
                // SAFETY: the proxy hands the tree over with the call.
                let #ident = unsafe { ::guatiao::library::kind::#take(#ident) };
            })
        })
        .collect();

    let reads = m.args.iter().map(|a| {
        let ident = &a.ident;
        match &a.kind {
            ArgKind::Scalar(_) | ArgKind::Object(_) => quote!(),
            ArgKind::ValueOwned | ArgKind::MapOwned | ArgKind::ListOwned => {
                let f = fail(quote!(__s));
                quote! {
                    let #ident = match #ident {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__s) => { #f }
                    };
                }
            }
            ArgKind::Str => {
                let f = fail(quote!(::guatiao::Status::from(__e)));
                quote! {
                    // The caller may be C: checked where it arrives.
                    let #ident = match ::core::str::from_utf8(#ident.into()) {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__e) => { #f }
                    };
                }
            }
            ArgKind::Bytes => {
                quote! {
                    let #ident: &[u8] = ::core::convert::From::from(#ident);
                }
            }
            ArgKind::ValueRef => {
                let f = fail(quote!(__s));
                quote! {
                    // SAFETY: the proxy passes a well-formed value.
                    let #ident = match unsafe { ::guatiao::library::kind::value_arg(#ident) } {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__s) => { #f }
                    };
                }
            }
            ArgKind::ValueOpt => quote! {
                // SAFETY: the proxy passes null or a well-formed value.
                let #ident = unsafe { ::guatiao::library::kind::value_opt(#ident) };
            },
            ArgKind::MapRef => {
                let f = fail(quote!(__s));
                quote! {
                    // SAFETY: the proxy passes a well-formed map.
                    let #ident = match unsafe { ::guatiao::library::kind::map_arg(#ident) } {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__s) => { #f }
                    };
                }
            }
            ArgKind::ValueMut | ArgKind::MapMut | ArgKind::ListMut => {
                let f = fail(quote!(__s));
                let read = match &a.kind {
                    ArgKind::ValueMut => quote!(value_mut_arg),
                    ArgKind::MapMut => quote!(map_mut_arg),
                    _ => quote!(list_mut_arg),
                };
                quote! {
                    // SAFETY: the proxy lends a well-formed tree for the call,
                    // reached through nothing else until it returns.
                    let #ident = match unsafe { ::guatiao::library::kind::#read(#ident) } {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__s) => { #f }
                    };
                }
            }
            ArgKind::Owned(ty) => {
                let f = fail(quote!(::guatiao::library::ProviderError::new(
                    ::guatiao::Status::GUATIAO_ERR_BAD_VALUE,
                    &__e.to_string(),
                )));
                let f2 = fail(quote!(__s));
                quote! {
                    // SAFETY: the proxy passes a well-formed value.
                    let #ident = match unsafe { ::guatiao::library::kind::value_arg(#ident) } {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__s) => { #f2 }
                    };
                    let #ident: #ty = match <#ty as ::guatiao::FromValue>::from_value(#ident) {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__e) => { #f }
                    };
                }
            }
            ArgKind::BytesMut => {
                quote! {
                    let #ident: &mut [u8] = ::core::convert::From::from(#ident);
                }
            }
            // Always fallible: the plan requires `Result` for a lent object.
            // Nothing is owned, so a refusal destroys nothing.
            ArgKind::BorrowMut(inner) => quote! {
                // SAFETY: the proxy passes an object lent for the call.
                let mut #ident = match unsafe { ::guatiao::library::kind::borrowed_mut_arg::<#inner>(#ident) } {
                    ::core::result::Result::Ok(__v) => __v,
                    // SAFETY: the proxy passes a writable error slot.
                    ::core::result::Result::Err(__e) => {
                        return unsafe { ::guatiao::library::kind::write_err(__err, __e) };
                    }
                };
            },
            ArgKind::Borrow(inner) => quote! {
                // SAFETY: the proxy passes an object lent for the call.
                let #ident = match unsafe { ::guatiao::library::kind::borrowed_arg::<#inner>(#ident) } {
                    ::core::result::Result::Ok(__v) => __v,
                    // SAFETY: the proxy passes a writable error slot.
                    ::core::result::Result::Err(__e) => {
                        return unsafe { ::guatiao::library::kind::write_err(__err, __e) };
                    }
                };
            },
        }
    });

    let call_args = m.args.iter().map(|a| {
        let ident = &a.ident;
        match &a.kind {
            ArgKind::BorrowMut(_) => quote!(&mut *#ident),
            ArgKind::Borrow(_) => quote!(&*#ident),
            _ => quote!(#ident),
        }
    });
    // A tree is built in the allocator the caller's carrier names, through
    // `<method>_in`; any other answer through the method itself.
    let call = if m.answers_tree() {
        let in_method = in_ident(method);
        quote!(__this.#in_method(#(#call_args,)* __alloc))
    } else {
        quote!(__this.#method(#(#call_args),*))
    };
    let alloc_read = matches!(
        m.ret,
        RetKind::Value | RetKind::Map | RetKind::List | RetKind::Owned(_)
    )
    .then(|| {
        quote! {
            // SAFETY: the proxy passes an initialised out slot, and the
            // allocator a carrier names outlives what is built through it.
            let __alloc = unsafe { ::guatiao::library::kind::return_alloc(__out) };
        }
    });

    // Who `ctx` is: a provider's `&T`, the cell an object was boxed in, or
    // the address of a lender's pointer.
    let this = match receiver {
        Receiver::Cell if m.mutable => quote! {
            // SAFETY: the table was built for `__T` and an object's `ctx`
            // is the cell `into_object` boxed; a `&mut self` slot is
            // reached through `&mut` to the one handle.
            let ::core::option::Option::Some(__this) =
                (unsafe { ::guatiao::library::kind::object_mut::<#table, __T>(__ctx) })
            else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
        },
        Receiver::Cell => quote! {
            // SAFETY: the table was built for `__T` and an object's `ctx`
            // is the cell `into_object` boxed; a `&self` slot is reached
            // through a shared borrow, shared across threads only when the
            // kind is `Sync`.
            let ::core::option::Option::Some(__this) =
                (unsafe { ::guatiao::library::kind::object_ref::<#table, __T>(__ctx) })
            else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
        },
        Receiver::Provider => quote! {
            // SAFETY: the table was built for `__T` and the descriptor's
            // `ctx` is a `&__T`.
            let ::core::option::Option::Some(__this) =
                (unsafe { ::guatiao::library::kind::ctx_ref::<__T>(__ctx) })
            else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
        },
        Receiver::LentMut if m.mutable => quote! {
            // SAFETY: a table `lent_mut()` built is lent with `ctx` the
            // address of the lender's `*mut dyn` for the call; a `&mut self`
            // slot is reached through `&mut` to the one `BorrowedMut`.
            let ::core::option::Option::Some(__this) =
                (unsafe { ::guatiao::library::kind::lent_mut::<dyn #trait_ident>(__ctx) })
            else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
        },
        Receiver::LentMut => quote! {
            // SAFETY: a table `lent_mut()` built is lent with `ctx` the
            // address of the lender's `*mut dyn`, read here as a `*const
            // dyn`; a `&self` slot is reached through a shared borrow of the
            // `BorrowedMut`, shared across threads only when the kind is
            // `Sync`.
            let ::core::option::Option::Some(__this) =
                (unsafe { ::guatiao::library::kind::lent_ref::<dyn #trait_ident>(__ctx) })
            else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
        },
        Receiver::Lent => quote! {
            // SAFETY: a table `lent()` built is lent with `ctx` the address
            // of the lender's `*const dyn` for the call; shared across
            // threads only when the kind is `Sync`.
            let ::core::option::Option::Some(__this) =
                (unsafe { ::guatiao::library::kind::lent_ref::<dyn #trait_ident>(__ctx) })
            else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
        },
    };

    // A tree is written once it lives in the carrier's allocator: a default
    // `<method>_in`, or one that ignores `alloc`, built it in this side's.
    let err_slot = if m.fallible {
        quote!(__err)
    } else {
        quote!(::core::ptr::null_mut())
    };
    let tree_out = |moved: TokenStream, answer: TokenStream| {
        quote! {
            match ::guatiao::library::kind::#moved(#answer, __alloc) {
                // SAFETY: the proxy passes a writable out slot.
                ::core::result::Result::Ok(__v) => unsafe { ::guatiao::library::kind::write_out(__out, __v) },
                // SAFETY: the proxy passes a writable error slot, or none.
                ::core::result::Result::Err(__e) => unsafe { ::guatiao::library::kind::write_err(#err_slot, __e) },
            }
        }
    };

    // What the implementation answered, as the status the shim returns.
    let write = |answer: TokenStream| match &m.ret {
        RetKind::Unit => quote! { { let _ = #answer; ::guatiao::Status::GUATIAO_OK } },
        RetKind::Value => tree_out(quote!(value_ret), answer),
        RetKind::Map => tree_out(quote!(map_ret), answer),
        RetKind::List => tree_out(quote!(list_ret), answer),
        RetKind::Scalar(_) => quote! {
            // SAFETY: the proxy passes a writable out slot.
            unsafe { ::guatiao::library::kind::write_out(__out, #answer) }
        },
        RetKind::Text => quote! {
            // SAFETY: the proxy passes a writable out slot.
            unsafe { ::guatiao::library::kind::write_out(__out, ::guatiao::Text::new(&#answer)) }
        },
        RetKind::Owned(ty) => quote! {
            match <#ty as ::guatiao::ToValue>::to_value_in(&#answer, __alloc) {
                // SAFETY: the proxy passes a writable out slot.
                ::core::result::Result::Ok(__v) => unsafe { ::guatiao::library::kind::write_out(__out, __v) },
                // SAFETY: the proxy passes a writable error slot.
                ::core::result::Result::Err(__e) => unsafe {
                    ::guatiao::library::kind::write_err(__err, ::guatiao::library::ProviderError::from(__e))
                },
            }
        },
        RetKind::Object(inner) => quote! {
            // SAFETY: the proxy passes a writable out slot.
            unsafe { ::guatiao::library::kind::object_out::<#inner>(__out, #answer) }
        },
    };
    let body = if m.fallible {
        let ok = write(quote!(__answer));
        quote! {
            match #call {
                ::core::result::Result::Ok(__answer) => { #ok }
                // SAFETY: the proxy passes a writable error slot.
                ::core::result::Result::Err(__e) => unsafe { ::guatiao::library::kind::write_err(__err, __e) },
            }
        }
    } else {
        write(call)
    };

    // Lent as `&dyn`, a `&mut self` method has no receiver it may use: the
    // slot refuses, after taking any object it was handed.
    if receiver == Receiver::Lent && m.mutable {
        let message = format!(
            "lent as `&dyn {trait_ident}`, which cannot call `{method}`: it takes `&mut self`"
        );
        let refuse = if m.fallible {
            quote! {
                // SAFETY: the caller passes a writable error slot.
                unsafe {
                    ::guatiao::library::kind::write_err(
                        __err,
                        ::guatiao::library::ProviderError::new(
                            ::guatiao::Status::GUATIAO_ERR_UNSUPPORTED,
                            #message,
                        ),
                    )
                }
            }
        } else {
            quote!(::guatiao::Status::GUATIAO_ERR_UNSUPPORTED)
        };
        return quote! {
            /// # Safety
            ///
            /// Called through the table `lent()` built.
            #[doc(hidden)]
            #[allow(unused_variables)]
            pub unsafe extern "C" fn #shim(
                __ctx: *mut ::core::ffi::c_void
                #(, #params)*
                #out_param
                #err_param
            ) -> ::guatiao::Status {
                ::guatiao::library::kind::catch(|| {
                    #(#tree_takes)*
                    #(#object_reads)*
                    #refuse
                })
            }
        };
    }

    let safety = match receiver {
        Receiver::Provider => quote! {
            /// Called through the table `of::<__T>()` built, with the `ctx`
            /// that table's provider declared: a `&__T`.
        },
        Receiver::Cell => quote! {
            /// Called through the table `of::<__T>()` built, with the `ctx`
            /// `into_object` boxed for a `__T`.
        },
        Receiver::LentMut | Receiver::Lent => quote! {
            /// Called through the lent table, with `ctx` the address of the
            /// lender's pointer, for the call.
        },
    };
    quote! {
        /// # Safety
        ///
        #safety
        #[doc(hidden)]
        pub unsafe extern "C" fn #shim #generics(
            __ctx: *mut ::core::ffi::c_void
            #(, #params)*
            #out_param
            #err_param
        ) -> ::guatiao::Status {
            ::guatiao::library::kind::catch(|| {
                #(#tree_takes)*
                #(#object_reads)*
                #this
                #(#reads)*
                #alloc_read
                #body
            })
        }
    }
}

/// The proxy method: reads its slot, marshals the arguments, calls, and
/// converts the answer back.
fn emit_proxy(table: &Ident, m: &MethodPlan) -> TokenStream {
    let method = &m.ident;
    let end = end_ident(&m.ident);
    let slot_ty = slot_type(m);
    let ret_ty = &m.ret_ty;
    let params: Vec<TokenStream> = m
        .args
        .iter()
        .map(|a| {
            let ident = &a.ident;
            let ty = &a.ty;
            quote!(#ident: #ty)
        })
        .collect();

    // An error on the proxy side: `Err` for a fallible method, a panic
    // for one that cannot fail (the failure is then a broken table or a
    // provider bug, which in-process would also have been a panic).
    let name = method.to_string();
    let fail = |e: TokenStream| {
        if m.fallible {
            quote! { return ::core::result::Result::Err(#e); }
        } else {
            quote! {
                ::core::panic!(
                    "guatiao: the provider failed `{}`, which cannot fail: {}",
                    #name, #e
                );
            }
        }
    };

    let handed_over = |a: &&ArgPlan| {
        matches!(
            a.kind,
            ArgKind::ValueOwned | ArgKind::MapOwned | ArgKind::ListOwned
        )
    };
    let in_order = m
        .args
        .iter()
        .filter(|a| !handed_over(a))
        .chain(m.args.iter().filter(handed_over));
    let marshals = in_order.map(|a| {
        let ident = &a.ident;
        let name = format_ident!("__arg_{}", ident);
        match &a.kind {
            ArgKind::Scalar(_) => quote! { let #name = #ident; },
            ArgKind::Str => quote! { let #name = ::guatiao::Str::new(#ident); },
            ArgKind::Bytes => quote! {
                let #name = ::guatiao::value::types::Bytes::new(#ident);
            },
            ArgKind::ValueRef => quote! { let #name = #ident as *const ::guatiao::Value; },
            ArgKind::MapRef => quote! { let #name = #ident as *const ::guatiao::Map; },
            // Handed over with the call: the callee frees it, so this side
            // forgets it. Marshalled after every argument that can fail.
            ArgKind::ValueOwned | ArgKind::MapOwned | ArgKind::ListOwned => {
                let held = format_ident!("__own_{}", ident);
                let ty = match &a.kind {
                    ArgKind::ValueOwned => quote!(::guatiao::Value),
                    ArgKind::MapOwned => quote!(::guatiao::Map),
                    _ => quote!(::guatiao::List),
                };
                quote! {
                    let mut #held = ::core::mem::ManuallyDrop::new(#ident);
                    let #name = &mut *#held as *mut #ty;
                }
            }
            // Lent for the call: the allocator it records now is where
            // whatever the callee adds is moved once it returns.
            ArgKind::ValueMut | ArgKind::MapMut | ArgKind::ListMut => {
                let home = format_ident!("__home_{}", ident);
                let ty = match &a.kind {
                    ArgKind::ValueMut => quote!(::guatiao::Value),
                    ArgKind::MapMut => quote!(::guatiao::Map),
                    _ => quote!(::guatiao::List),
                };
                quote! {
                    let #home = ::guatiao::library::kind::lent_home(&*#ident);
                    let #name = &mut *#ident as *mut #ty;
                }
            }
            ArgKind::ValueOpt => quote! {
                let #name = #ident.map_or(::core::ptr::null(), |__v| __v as *const ::guatiao::Value);
            },
            ArgKind::Owned(ty) => {
                let f = fail(quote!(::guatiao::library::ProviderError::from(__e)));
                quote! {
                    let #name = match <#ty as ::guatiao::ToValue>::to_value_in(&#ident, ::guatiao::Alloc::rust()) {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__e) => { #f }
                    };
                    let #name = &#name as *const ::guatiao::Value;
                }
            }
            ArgKind::BytesMut => quote! {
                let #name = ::guatiao::library::BytesMut::new(#ident);
            },
            // Ownership crosses with the call: the callee destroys it.
            ArgKind::Object(_) => quote! { let #name = #ident.into_raw(); },
            // Lent for the call: the lent table's slots read `ctx` as the
            // address of this pointer, and both stay on this stack.
            ArgKind::BorrowMut(inner) => {
                let slot = format_ident!("__lent_{}", ident);
                let raw = format_ident!("__raw_{}", ident);
                quote! {
                    let #slot: *mut _ = #ident;
                    let #raw = ::guatiao::library::kind::lend_mut::<#inner>(
                        ::core::ptr::addr_of!(#slot).cast_mut().cast(),
                    );
                    let #name = &#raw as *const ::guatiao::library::ObjectRaw;
                }
            }
            ArgKind::Borrow(inner) => {
                let slot = format_ident!("__lent_{}", ident);
                let raw = format_ident!("__raw_{}", ident);
                quote! {
                    let #slot: *const _ = #ident;
                    let #raw = ::guatiao::library::kind::lend::<#inner>(
                        ::core::ptr::addr_of!(#slot).cast_mut().cast(),
                    );
                    let #name = &#raw as *const ::guatiao::library::ObjectRaw;
                }
            }
        }
    });
    let arg_names = m.args.iter().map(|a| format_ident!("__arg_{}", a.ident));

    // A tree crosses through a carrier: an empty container recording the
    // allocator the answer is wanted in, which the shim builds into.
    let carrier = |ty: TokenStream| {
        quote! {
            let mut __out = <#ty as ::guatiao::library::kind::Carrier>::carrier(__alloc);
        }
    };
    let out_init = match &m.ret {
        RetKind::Unit => quote!(),
        RetKind::Scalar(ty) => quote! { let mut __out: #ty = ::core::default::Default::default(); },
        RetKind::Value | RetKind::Owned(_) => carrier(quote!(::guatiao::Value)),
        RetKind::Map => carrier(quote!(::guatiao::Map)),
        RetKind::List => carrier(quote!(::guatiao::List)),
        RetKind::Text => quote! { let mut __out = ::guatiao::Text::new(""); },
        RetKind::Object(_) => quote! { let mut __out = ::guatiao::library::ObjectRaw::null(); },
    };
    let out_arg = if matches!(m.ret, RetKind::Unit) {
        quote!()
    } else {
        quote!(, &mut __out)
    };
    let err_init = m
        .fallible
        .then(|| quote! { let mut __err = ::guatiao::library::ProviderError::none(); });
    let err_arg = m.fallible.then(|| quote!(, &mut __err));

    let on_status = if m.fallible {
        quote! {
            if __status != ::guatiao::Status::GUATIAO_OK {
                return ::core::result::Result::Err(::guatiao::library::kind::take_err(__err, __status));
            }
        }
    } else {
        let f = fail(quote!(::core::format_args!("{:?}", __status)));
        quote! {
            if __status != ::guatiao::Status::GUATIAO_OK {
                #f
            }
        }
    };

    let answer = match &m.ret {
        // A unit answer in an infallible method is the block's own end;
        // spelling `()` there is what clippy calls an unneeded unit.
        RetKind::Unit if !m.fallible => quote!(),
        RetKind::Unit => quote!(()),
        RetKind::Scalar(_) => quote!(__out),
        // A tree the library built is moved into this side's allocator
        // before the caller sees it, so nothing it holds needs the
        // library's allocator once the library is gone.
        RetKind::Value | RetKind::Map | RetKind::List => {
            let f = fail(quote!(__e));
            let ret = match &m.ret {
                RetKind::Value => quote!(value_ret),
                RetKind::Map => quote!(map_ret),
                _ => quote!(list_ret),
            };
            quote! {
                match ::guatiao::library::kind::#ret(__out, __alloc) {
                    ::core::result::Result::Ok(__v) => __v,
                    ::core::result::Result::Err(__e) => { #f }
                }
            }
        }
        RetKind::Text => {
            let f = fail(quote!(__e));
            quote! {
                match ::guatiao::library::kind::text_ret(__out) {
                    ::core::result::Result::Ok(__v) => ::guatiao::__private::alloc::string::ToString::to_string(&*__v),
                    ::core::result::Result::Err(__e) => { #f }
                }
            }
        }
        RetKind::Owned(ty) => {
            let f = fail(quote!(::guatiao::library::ProviderError::new(
                ::guatiao::Status::GUATIAO_ERR_BAD_VALUE,
                &__e.to_string(),
            )));
            quote! {
                match <#ty as ::guatiao::FromValue>::from_value(&__out) {
                    ::core::result::Result::Ok(__v) => __v,
                    ::core::result::Result::Err(__e) => { #f }
                }
            }
        }
        RetKind::Object(inner) => {
            // The object holds what this proxy holds on the library that
            // made it, through a shared borrow of the receiver.
            let from = if m.mutable {
                quote!(&*self)
            } else {
                quote!(self)
            };
            // Always fallible: the plan requires `Result` for an object.
            quote! {
                // SAFETY: the shim wrote an object it handed across, or left
                // the slot null.
                match unsafe { ::guatiao::library::kind::object_ret::<#inner, _>(__out, #from) } {
                    ::core::result::Result::Ok(__v) => __v,
                    ::core::result::Result::Err(__e) => { return ::core::result::Result::Err(__e); }
                }
            }
        }
    };
    let answer = if m.fallible {
        quote!(::core::result::Result::Ok(#answer))
    } else {
        answer
    };

    // The allocator the answer is wanted in: the one `<method>_in` was
    // handed for a tree, this side's own for a value decoded on arrival.
    let alloc_param = m.alloc_param();
    let alloc_init = if m.answers_tree() {
        Some(quote! { let __alloc = #alloc_param; })
    } else {
        matches!(m.ret, RetKind::Owned(_))
            .then(|| quote! { let __alloc = ::guatiao::Alloc::rust(); })
    };
    // A tree lent mutably is settled after the call, OK or not: whatever
    // the callee put into it is moved into the allocator it recorded, so
    // nothing in it needs the callee's. A refused call's own error wins.
    let lent: Vec<&ArgPlan> = m.args.iter().filter(|a| a.kind.is_lent_tree()).collect();
    let settle = (!lent.is_empty()).then(|| {
        let settles = lent.iter().map(|a| {
            let ident = &a.ident;
            let home = format_ident!("__home_{}", ident);
            quote! { .and(::guatiao::library::kind::lent_settle(&mut *#ident, #home)) }
        });
        quote! {
            let __settled: ::core::result::Result<(), ::guatiao::library::ProviderError> =
                ::core::result::Result::Ok(()) #(#settles)*;
        }
    });
    let settled = (!lent.is_empty()).then(|| {
        let f = fail(quote!(__e));
        quote! {
            if let ::core::result::Result::Err(__e) = __settled {
                #f
            }
        }
    });
    let call = quote! {
        #(#marshals)*
        #out_init
        #err_init
        // SAFETY: the slot's own signature, read under the table's size;
        // every pointer is a live local or a borrowed argument.
        let __status = unsafe { __f(self.ctx() #(, #arg_names)* #out_arg #err_arg) };
        #settle
        #on_status
        #settled
        #answer
    };

    let body = match &m.default {
        // A required slot: validation established it is present.
        None => quote! {
            // SAFETY: the slot at that offset has this type in `#table`.
            let __f: #slot_ty = unsafe { self.slot(::core::mem::offset_of!(#table, #method), #table::#end()) }
                .expect("validated: a required slot is present and non-null");
            #call
        },
        // An appended slot: absent from an older table, so the default
        // body runs, exactly as it would on a type implementing the trait,
        // and a tree it answers is moved into the allocator asked for.
        Some(block) => {
            let fallback = if m.answers_tree() {
                let ret = match &m.ret {
                    RetKind::Value => quote!(value_ret),
                    RetKind::Map => quote!(map_ret),
                    _ => quote!(list_ret),
                };
                let rehome = if m.fallible {
                    quote!(__out.and_then(|__v| ::guatiao::library::kind::#ret(__v, __alloc)))
                } else {
                    let f = fail(quote!(__e));
                    quote! {
                        match ::guatiao::library::kind::#ret(__out, __alloc) {
                            ::core::result::Result::Ok(__v) => __v,
                            ::core::result::Result::Err(__e) => { #f }
                        }
                    }
                };
                quote! {{
                    let __out = (|| #ret_ty #block)();
                    #rehome
                }}
            } else {
                quote!(#block)
            };
            quote! {
                // SAFETY: the slot at that offset has this type in `#table`.
                match unsafe { self.slot::<#slot_ty>(::core::mem::offset_of!(#table, #method), #table::#end()) } {
                    ::core::option::Option::Some(__f) => { #call }
                    ::core::option::Option::None => #fallback
                }
            }
        }
    };

    let receiver = if m.mutable {
        quote!(&mut self)
    } else {
        quote!(&self)
    };
    if !m.answers_tree() {
        return quote! {
            fn #method(#receiver #(, #params)*) #ret_ty {
                #alloc_init
                #body
            }
        };
    }
    // A tree: the call with the caller's allocator, and the method itself
    // asking for this side's.
    let in_method = in_ident(method);
    let names = m.args.iter().map(|a| &a.ident);
    quote! {
        fn #in_method(#receiver #(, #params)*, #alloc_param: ::guatiao::Alloc) #ret_ty {
            #alloc_init
            #body
        }

        fn #method(#receiver #(, #params)*) #ret_ty {
            self.#in_method(#(#names,)* ::guatiao::Alloc::rust())
        }
    }
}

/// One method of the trait on an `Instance`: the same call on the proxy
/// the instance derefs to.
fn emit_forward(m: &MethodPlan) -> TokenStream {
    let method = &m.ident;
    let ret_ty = &m.ret_ty;
    let params = m.args.iter().map(|a| {
        let ident = &a.ident;
        let ty = &a.ty;
        quote!(#ident: #ty)
    });
    let names: Vec<&Ident> = m.args.iter().map(|a| &a.ident).collect();
    let in_forward = m.answers_tree().then(|| {
        let in_method = in_ident(method);
        let alloc = m.alloc_param();
        let params = m.args.iter().map(|a| {
            let ident = &a.ident;
            let ty = &a.ty;
            quote!(#ident: #ty)
        });
        quote! {
            fn #in_method(&self #(, #params)*, #alloc: ::guatiao::Alloc) #ret_ty {
                (**self).#in_method(#(#names,)* #alloc)
            }
        }
    });
    quote! {
        fn #method(&self #(, #params)*) #ret_ty {
            (**self).#method(#(#names),*)
        }

        #in_forward
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_str(attr: &str, item: &str) -> Result<String, String> {
        let attr: TokenStream = attr.parse().unwrap();
        let item: TokenStream = item.parse().unwrap();
        match try_expand(attr, item) {
            Ok(t) => Ok(t.to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    const GREETER: &str = r#"
        pub trait Greeter: Send + Sync {
            fn greet(&self, name: &str) -> Result<String, ProviderError>;
            fn count(&self, bytes: &[u8], flag: bool) -> i64 { bytes.len() as i64 }
        }
    "#;

    #[test]
    fn a_two_method_trait_expands_to_parseable_items() {
        let out = expand_str("", GREETER).expect("expands");
        let file: syn::File = syn::parse_str(&out).expect("the expansion parses as Rust");
        let names: Vec<String> = file
            .items
            .iter()
            .filter_map(|i| match i {
                syn::Item::Trait(t) => Some(format!("trait {}", t.ident)),
                syn::Item::Struct(s) => Some(format!("struct {}", s.ident)),
                syn::Item::Impl(i) => Some(format!(
                    "impl {}",
                    i.trait_
                        .as_ref()
                        .map(|(_, p, _)| quote!(#p).to_string())
                        .unwrap_or_else(|| "inherent".to_string())
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            [
                "trait Greeter",
                "struct GreeterVtable",
                "impl inherent",
                "impl :: guatiao :: library :: Kind",
                "impl :: guatiao :: library :: kind :: Lend",
                "impl Greeter",
                "impl :: core :: convert :: From < :: guatiao :: library :: Remote < dyn Greeter > >",
                "impl Greeter",
                "impl :: core :: convert :: From < :: guatiao :: library :: Instance < dyn Greeter > >",
            ]
        );
        // The second `impl Greeter` is the instance's, forwarding every
        // method, the defaulted one included, to the proxy it derefs to.
        assert!(out.contains("for :: guatiao :: library :: Instance < dyn Greeter >"));
        assert!(out.contains("(* * self) . greet (name)"), "{out}");
        assert!(out.contains("(* * self) . count (bytes , flag)"), "{out}");
        assert!(out.contains("const NAME : & 'static str = \"greeter\""));
        // Only the required method is hashed, normalised, behind the
        // shape -- and written as a literal on the table, where a C
        // header generator can read it.
        let literal = fnv1a("provider;greet(&str)->Result<String,ProviderError>");
        assert!(
            out.contains(&format!("pub const FLOOR_HASH : u32 = {literal} ;")),
            "{out}"
        );
        assert!(out.contains("const FLOOR_HASH : u32 = GreeterVtable :: FLOOR_HASH ;"));
        assert!(out.contains("(\"greet\" , GreeterVtable :: greet_end ())"));
        assert!(out.contains("const OBJECT : bool = false"));
        assert!(
            !out.contains("destroy"),
            "a provider kind has no destroy slot"
        );
    }

    const CONVERSATION: &str = r#"
        pub trait Conversation: Send {
            fn say(&mut self, what: &str) -> Result<(), ProviderError>;
            fn read(&mut self, dst: &mut [u8]) -> i64;
            fn turns(&self) -> i64 { 0 }
        }
    "#;

    #[test]
    fn an_object_kind_carries_destroy_first_and_gains_into_object() {
        let out = expand_str("object", CONVERSATION).expect("expands");
        let file: syn::File = syn::parse_str(&out).expect("the expansion parses as Rust");
        // The trait was emitted with `into_object` appended, off the table.
        let tr = file
            .items
            .iter()
            .find_map(|i| match i {
                syn::Item::Trait(t) => Some(t),
                _ => None,
            })
            .expect("the trait");
        let names: Vec<String> = tr
            .items
            .iter()
            .filter_map(|i| match i {
                syn::TraitItem::Fn(f) => Some(f.sig.ident.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["say", "read", "turns", "into_object"]);
        // The table: header, destroy, then the slots in order.
        let table = file
            .items
            .iter()
            .find_map(|i| match i {
                syn::Item::Struct(s) if s.ident == "ConversationVtable" => Some(s),
                _ => None,
            })
            .expect("the table");
        let fields: Vec<String> = table
            .fields
            .iter()
            .map(|f| f.ident.as_ref().unwrap().to_string())
            .collect();
        assert_eq!(fields, ["header", "destroy", "say", "read", "turns"]);
        assert!(out.contains("const OBJECT : bool = true"));
        assert!(!out.contains("Instance"), "an object kind has no instances");
        assert!(out.contains("(\"destroy\" , ConversationVtable :: destroy_end ())"));
        let literal = fnv1a("object;say(&str)->Result<(),ProviderError>;read(&mut[u8])->i64");
        assert!(
            out.contains(&format!("pub const FLOOR_HASH : u32 = {literal} ;")),
            "{out}"
        );
        // The known FNV-1a vectors, so the constant here matches the
        // runtime's (which `kind_roundtrip` checks from the other side).
        assert_eq!(fnv1a(""), 0x811c_9dc5);
        assert_eq!(fnv1a("a"), 0xe40c_292c);
        // `&mut self` reaches the proxy; the out-buffer crosses as BytesMut.
        assert!(out.contains("fn say (& mut self , what : & str)"));
        assert!(out.contains("dst : :: guatiao :: library :: BytesMut"));
        assert!(out.contains("destroy_object :: < Self , __T >"));
        // A `&mut self` slot takes the value exclusively, a `&self` one
        // shared: an object of a `Sync` kind is shared between threads.
        assert!(
            shim_body(&out, "__guatiao_shim_say")
                .contains("object_mut :: < ConversationVtable , __T >")
        );
        let turns = shim_body(&out, "__guatiao_shim_turns");
        assert!(
            turns.contains("object_ref :: < ConversationVtable , __T >"),
            "{turns}"
        );
        assert!(!turns.contains("object_mut"), "{turns}");
    }

    #[test]
    fn an_object_crosses_as_an_argument_and_as_a_return() {
        let out = expand_str(
            "",
            "pub trait Greeter: Send + Sync { fn start(&self, listener: Object<dyn Listener>) -> Result<Object<dyn Conversation>, ProviderError>; }",
        )
        .expect("expands");
        assert!(out.contains("listener : :: guatiao :: library :: ObjectRaw"));
        assert!(out.contains("out : * mut :: guatiao :: library :: ObjectRaw"));
        assert!(out.contains("object_arg :: < dyn Listener >"));
        assert!(out.contains("object_out :: < dyn Conversation >"));
        assert!(
            out.contains("object_ret :: < dyn Conversation , _ > (__out , self)"),
            "the returned object holds what the proxy holds: {out}"
        );
        assert!(out.contains("let __arg_listener = listener . into_raw ()"));

        // From a `&mut self` method, through a shared reborrow.
        let out = expand_str(
            "object",
            "pub trait Session: Send { fn fork(&mut self) -> Result<Object<dyn Session>, ProviderError>; }",
        )
        .expect("expands");
        assert!(
            out.contains("object_ret :: < dyn Session , _ > (__out , & * self)"),
            "{out}"
        );
    }

    #[test]
    fn a_lent_object_crosses_as_a_pointer_to_its_raw_form() {
        let out = expand_str(
            "",
            "pub trait Greeter: Send + Sync { fn f(&self, l: &mut dyn Listener, g: &(dyn Gauge)) -> Result<(), ProviderError>; }",
        )
        .expect("expands");
        // In C, a `const guatiao_object *` on the caller's stack.
        assert!(
            out.contains("l : * const :: guatiao :: library :: ObjectRaw"),
            "{out}"
        );
        assert!(
            out.contains("g : * const :: guatiao :: library :: ObjectRaw"),
            "{out}"
        );
        // The shim borrows it, owning nothing, and lends it on.
        assert!(
            out.contains("borrowed_mut_arg :: < dyn Listener >"),
            "{out}"
        );
        assert!(out.contains("borrowed_arg :: < dyn Gauge >"), "{out}");
        assert!(out.contains("__this . f (& mut * l , & * g)"), "{out}");
        // The proxy lends from its own stack.
        assert!(out.contains("let __lent_l : * mut _ = l ;"), "{out}");
        assert!(out.contains("lend_mut :: < dyn Listener >"), "{out}");
        assert!(out.contains("lend :: < dyn Gauge >"), "{out}");
        // And the shape is checked where the lent kind's impl is visible.
        assert!(
            out.contains(
                "< dyn Listener as :: guatiao :: library :: kind :: Lend > :: LENT_MUT . is_some ()"
            ),
            "{out}"
        );
        assert!(
            out.contains(
                "< dyn Gauge as :: guatiao :: library :: kind :: Lend > :: LENT . is_some ()"
            ),
            "{out}"
        );
        // Hashed as spelled, canonically.
        assert_eq!(
            floor_hash(
                "",
                "pub trait Greeter: Send + Sync { fn f(&self, l: &mut dyn Listener, g: &(dyn Gauge)) -> Result<(), ProviderError>; }"
            ),
            fnv1a("provider;f(&mutdynListener,&dynGauge)->Result<(),ProviderError>")
        );
    }

    #[test]
    fn an_object_kind_carries_its_lent_tables() {
        let out = expand_str("object", CONVERSATION).expect("expands");
        assert!(out.contains("pub const fn lent_mut () -> Self"), "{out}");
        assert!(
            out.contains("pub unsafe extern \"C\" fn __guatiao_lent_destroy"),
            "{out}"
        );
        // Lent `&mut`, a `&mut self` slot takes the lender's object
        // exclusively and a `&self` one shared.
        let say = shim_body(&out, "__guatiao_lentmut__say");
        assert!(
            say.contains("lent_mut :: < dyn Conversation > (__ctx)"),
            "{say}"
        );
        let turns = shim_body(&out, "__guatiao_lentmut__turns");
        assert!(
            turns.contains("lent_ref :: < dyn Conversation > (__ctx)"),
            "{turns}"
        );
        assert!(!turns.contains("lent_mut ::"), "{turns}");
        // `turns(&self)`: it can be lent shared, and there `say` refuses.
        assert!(out.contains("pub const fn lent () -> Self"), "{out}");
        let turns = shim_body(&out, "__guatiao_lent__turns");
        assert!(
            turns.contains("lent_ref :: < dyn Conversation > (__ctx)"),
            "{turns}"
        );
        let say = shim_body(&out, "__guatiao_lent__say");
        assert!(say.contains("GUATIAO_ERR_UNSUPPORTED"), "{say}");
        assert!(
            out.contains("unsafe impl :: guatiao :: library :: kind :: Lend for dyn Conversation"),
            "{out}"
        );
        assert!(out.contains("const LENT_MUT :"), "{out}");
        assert!(out.contains("const LENT :"), "{out}");
        // Every method `&mut self`: nothing to call through `&dyn`.
        let out = expand_str("object", "pub trait W: Send { fn w(&mut self) -> i64; }").unwrap();
        assert!(out.contains("const LENT_MUT :"), "{out}");
        assert!(!out.contains("const LENT :"), "{out}");
        assert!(!out.contains("fn lent ()"), "{out}");
        // A provider kind is never lent: it is `Lend` with no tables, so a
        // lent argument of one meets the assertion's sentence.
        let out = expand_str("", GREETER).unwrap();
        assert!(!out.contains("LENT"), "{out}");
        assert!(
            out.contains("unsafe impl :: guatiao :: library :: kind :: Lend for dyn Greeter { }"),
            "{out}"
        );
    }

    /// The text of one generated shim, from its name to the next item.
    fn shim_body<'a>(out: &'a str, name: &str) -> &'a str {
        let start = out
            .find(&format!("fn {name}"))
            .unwrap_or_else(|| panic!("no shim `{name}` in {out}"));
        let rest = &out[start..];
        let end = rest[1..].find("pub ").map_or(rest.len(), |i| i + 1);
        &rest[..end]
    }

    /// Every function the table's inherent impl defines, by name.
    fn table_fns(out: &str, table: &str) -> Vec<String> {
        let file: syn::File = syn::parse_str(out).expect("the expansion parses as Rust");
        file.items
            .iter()
            .filter_map(|i| match i {
                syn::Item::Impl(imp) if imp.trait_.is_none() => match &*imp.self_ty {
                    Type::Path(p) if p.path.is_ident(table) => Some(imp),
                    _ => None,
                },
                _ => None,
            })
            .flat_map(|imp| &imp.items)
            .filter_map(|item| match item {
                syn::ImplItem::Fn(f) => Some(f.sig.ident.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn no_two_methods_generate_the_same_name() {
        // `x` lent `&mut` and `mut_x` lent shared, and every other pair
        // whose names a plain `_` join would run together.
        let out = expand_str(
            "object",
            "pub trait Pair: Send { fn x(&self) -> i64; fn mut_x(&mut self) -> i64; \
             fn lent_x(&self) -> i64; fn shim_x(&self) -> i64; fn _x(&self) -> i64; }",
        )
        .expect("expands");
        let mut fns = table_fns(&out, "PairVtable");
        let all = fns.len();
        fns.sort();
        fns.dedup();
        assert_eq!(fns.len(), all, "a name is generated twice: {fns:?}");
        assert!(fns.contains(&"__guatiao_lentmut__x".to_string()), "{fns:?}");
        assert!(
            fns.contains(&"__guatiao_lent__mut_x".to_string()),
            "{fns:?}"
        );
    }

    #[test]
    fn a_provider_kind_may_name_a_method_destroy() {
        // Only an object kind's table has a `destroy` slot of its own.
        let out = expand_str(
            "",
            "pub trait Pool: Send + Sync { fn destroy(&self) -> i64; }",
        )
        .expect("expands");
        assert!(
            out.contains("pub destroy : :: core :: option :: Option"),
            "{out}"
        );
        assert!(!out.contains("__guatiao_destroy"), "{out}");
        assert!(
            out.contains("(\"destroy\" , PoolVtable :: destroy_end ())"),
            "{out}"
        );
    }

    #[test]
    fn a_type_of_the_callers_crosses_by_value_whatever_its_last_name() {
        for ty in [
            "proto::String",
            "foreign::Map",
            "model::Value",
            "model::List",
            "mine::ProviderError",
            "json::Object",
            "json::Object<String>",
        ] {
            let out = expand_str(
                "",
                &format!(
                    "pub trait Echo: Send + Sync {{ fn take(&self, s: {ty}) -> Result<i64, ProviderError>; }}"
                ),
            )
            .unwrap_or_else(|e| panic!("{ty} is refused: {e}"));
            let read: Type = syn::parse_str(ty).unwrap();
            let read = quote!(#read).to_string();
            assert!(
                out.contains(&format!("< {read} as :: guatiao :: FromValue >")),
                "{ty}: {out}"
            );
        }
        // Borrowed, returned or as the error, such a path can only be
        // meant as guatiao's or the standard type, and is still refused
        // (`kind_rejects_by_name`); so is `other::Object<dyn K>`.
    }

    /// With `crate = <path>` nothing is rooted at `::guatiao`: a consumer
    /// reaching the crate through a re-export has no such name.
    #[test]
    fn a_crate_path_reroots_every_generated_path() {
        // Every way an argument and a return can cross.
        const EVERYTHING: &str = "pub trait Greeter: Send + Sync { \
            fn a(&self, s: &str, b: &[u8], v: &Value, o: Option<&Value>, m: &Map, n: i64, \
                c: Config, t: Value, u: Map, w: List, x: &mut Map, y: &mut List, \
                z: &mut Value) -> Result<Map, ProviderError>; \
            fn b(&self) -> Result<List, ProviderError>; \
            fn c(&self) -> Result<Value, ProviderError>; \
            fn d(&self) -> Result<Reply, ProviderError>; \
            fn e(&self, l: Object<dyn Listener>) \
                -> Result<Object<dyn Conversation>, ProviderError>; \
            fn f(&self) -> i64; \
            fn h(&self, l: &mut dyn Listener, c: &dyn Conversation) -> Result<(), ProviderError>; \
            fn g(&self) -> String { String::new() } }";
        for (attr, item) in [
            ("crate = ::kinds::guatiao", EVERYTHING),
            ("object, crate = ::kinds::guatiao", CONVERSATION),
        ] {
            let out = expand_str(attr, item).expect("expands");
            assert!(out.contains(":: kinds :: guatiao ::"), "{out}");
            let rest = out.replace(":: kinds :: guatiao", "");
            assert!(
                !rest.contains(":: guatiao"),
                "a path is still rooted at `::guatiao`: {rest}"
            );
        }
    }

    /// The expansion of `item`, against `src/snapshots/<name>.expected.rs`;
    /// on a difference, the new one is written beside it as
    /// `<name>.actual.rs`.
    fn matches_snapshot(name: &str, item: &str, expected: &str) {
        let out = expand_str("", item).expect("expands");
        let file: syn::File = syn::parse_str(&out).unwrap();
        let pretty = prettyplease::unparse(&file);
        if pretty != expected {
            let path = format!(
                "{}/src/snapshots/{name}.actual.rs",
                env!("CARGO_MANIFEST_DIR")
            );
            std::fs::write(path, &pretty).unwrap();
            panic!(
                "the expansion changed; compare src/snapshots/{name}.actual.rs with {name}.expected.rs and move it over if the change is intended"
            );
        }
    }

    #[test]
    fn the_expansion_matches_the_snapshot() {
        matches_snapshot(
            "greeter",
            GREETER,
            include_str!("snapshots/greeter.expected.rs"),
        );
    }

    /// Every tree answer: `<method>_in` on the trait, a shim building in
    /// the carried allocator, a proxy writing the carrier, and an appended
    /// slot's default moved into the allocator asked for.
    #[test]
    fn a_tree_answer_expands_to_its_snapshot() {
        const TREES: &str = r#"
            pub trait Trees: Send + Sync {
                fn map(&self, name: &str) -> Result<Map, ProviderError>;
                fn value(&self) -> Value;
                fn list(&self) -> List { List::new() }
            }
        "#;
        matches_snapshot("trees", TREES, include_str!("snapshots/trees.expected.rs"));
    }

    /// Trees by value: taken before anything can return in the shim,
    /// handed over last and forgotten in the proxy.
    #[test]
    fn a_tree_argument_expands_to_its_snapshot() {
        const MOVES: &str = r#"
            pub trait Moves: Send + Sync {
                fn keep(&self, v: Value, name: &str, m: Map) -> Result<i64, ProviderError>;
                fn take(&self, l: List) -> i64;
            }
        "#;
        matches_snapshot("moves", MOVES, include_str!("snapshots/moves.expected.rs"));
    }

    /// Trees lent mutably: the shim borrows the caller's node, the proxy
    /// reads where the tree lives before the call and moves whatever the
    /// callee built elsewhere there after it.
    #[test]
    fn a_lent_tree_expands_to_its_snapshot() {
        const LENDS: &str = r#"
            pub trait Lends: Send + Sync {
                fn check(&self, config: &mut Map) -> Result<bool, ProviderError>;
                fn fill(&self, v: &mut Value, l: &mut List, name: &str) -> i64;
            }
        "#;
        matches_snapshot("lends", LENDS, include_str!("snapshots/lends.expected.rs"));
    }

    #[test]
    fn a_lent_tree_crosses_as_a_pointer_without_const_and_hashes_apart() {
        let out = expand_str(
            "",
            "pub trait G: Send + Sync { fn check(&self, config: &mut Map) -> Result<bool, ProviderError>; }",
        )
        .expect("expands");
        assert!(out.contains("config : * mut :: guatiao :: Map"), "{out}");
        assert!(out.contains("map_mut_arg (config)"), "{out}");
        assert!(out.contains("lent_home (& * config)"), "{out}");
        assert!(
            out.contains("lent_settle (& mut * config , __home_config)"),
            "{out}"
        );
        // The slot says so, for a C reader: a handed-over tree is the
        // same pointer type.
        assert!(
            out.contains("Lent for the call, not handed over: `config`."),
            "{out}"
        );
        let hash = |ty: &str| {
            floor_hash(
                "",
                &format!(
                    "pub trait G: Send + Sync {{ fn check(&self, config: {ty}) -> Result<bool, ProviderError>; }}"
                ),
            )
        };
        assert_eq!(
            hash("&mut Map"),
            fnv1a("provider;check(&mutMap)->Result<bool,ProviderError>")
        );
        assert_eq!(hash("&mut ::guatiao::Map"), hash("&mut Map"));
        assert_ne!(hash("&mut Map"), hash("&Map"));
        assert_ne!(hash("&mut Map"), hash("Map"));
        // On an object kind too: the borrow is of the argument, not of
        // `self`, and the lent tables carry it.
        let out = expand_str(
            "object",
            "pub trait S: Send { fn fill(&mut self, v: &mut Value) -> i64; }",
        )
        .expect("expands");
        assert!(
            shim_body(&out, "__guatiao_lentmut__fill").contains("value_mut_arg (v)"),
            "{out}"
        );
    }

    #[test]
    fn a_method_named_like_an_added_one_is_refused() {
        let why = expand_str(
            "",
            "pub trait T: Send + Sync { fn map(&self) -> Map; fn map_in(&self) -> i64; }",
        )
        .expect_err("refused");
        assert!(why.contains("adds `map_in` beside `map`"), "{why}");
        // Not a tree: nothing is added, so the name is free.
        expand_str(
            "",
            "pub trait T: Send + Sync { fn count(&self) -> i64; fn count_in(&self) -> i64; }",
        )
        .expect("expands");
        // An argument named `alloc` leaves the added parameter `alloc_`.
        let out = expand_str(
            "",
            "pub trait T: Send + Sync { fn map(&self, alloc: i64) -> Map; }",
        )
        .expect("expands");
        assert!(out.contains("alloc_ : :: guatiao :: Alloc"), "{out}");
    }

    /// The literal the expansion writes as the table's hash.
    fn floor_hash(attr: &str, item: &str) -> u32 {
        let out = expand_str(attr, item).expect("expands");
        let (_, rest) = out
            .split_once("pub const FLOOR_HASH : u32 = ")
            .expect("the hash literal");
        rest.split_once(' ').unwrap().0.parse().unwrap()
    }

    #[test]
    fn the_hash_reads_std_and_guatiao_paths_as_their_bare_names() {
        let spelled = |ret: &str| {
            floor_hash(
                "",
                &format!("pub trait P: Send + Sync {{ fn channels(&self) -> {ret}; }}"),
            )
        };
        let bare = spelled("Result<Vec<Channel>, ProviderError>");
        assert_eq!(
            bare,
            fnv1a("provider;channels()->Result<Vec<Channel>,ProviderError>")
        );
        for ret in [
            "Result<std::vec::Vec<Channel>, ProviderError>",
            "Result<Vec<Channel>, guatiao::library::ProviderError>",
            "::core::result::Result<::alloc::vec::Vec<Channel>, ::guatiao::library::ProviderError>",
        ] {
            assert_eq!(spelled(ret), bare, "{ret}");
        }
        // A type of the caller's is hashed as written: a proc macro cannot
        // see that two names describe one wire shape.
        assert_ne!(spelled("Result<Vec<RenamedChannel>, ProviderError>"), bare);
        assert_ne!(spelled("Result<Vec<model::Channel>, ProviderError>"), bare);
        // The `crate =` root is guatiao's too.
        assert_eq!(
            floor_hash(
                "crate = ::kinds::guatiao",
                "pub trait P: Send + Sync { fn channels(&self) -> Result<Vec<Channel>, ::kinds::guatiao::library::ProviderError>; }",
            ),
            bare
        );
        // Arguments as well as returns, and a lifetime is not part of it.
        assert_eq!(
            floor_hash(
                "",
                "pub trait P: Send + Sync { fn f(&self, v: &'static ::guatiao::Value, m: &guatiao::Map) -> i64; }"
            ),
            fnv1a("provider;f(&Value,&Map)->i64")
        );
    }

    /// The shim's own parameters are `__ctx`, `__out` and `__err`, so an
    /// argument may take any of the plain names; in the slot's type, which
    /// a C header renders, such an argument gains a `_`.
    #[test]
    fn arguments_named_like_the_glue_s_parameters_cross() {
        let out = expand_str(
            "",
            "pub trait G: Send + Sync { fn f(&self, ctx: i64, out: &str, err: bool) -> Result<String, ProviderError>; }",
        )
        .expect("expands");
        assert!(out.contains("__ctx : * mut :: core :: ffi :: c_void , ctx : i64 , out : :: guatiao :: Str , err : bool , __out"), "{out}");
        assert!(
            out.contains("ctx_ : i64 , out_ : :: guatiao :: Str , err_ : bool , out : * mut"),
            "{out}"
        );
        // Every spelling of guatiao's and the standard types is read.
        for (spelled, shape) in [
            ("&::guatiao::Value", "value_arg"),
            ("&guatiao::value::types::Map", "map_arg"),
            ("::core::option::Option<&guatiao::Value>", "value_opt"),
        ] {
            let out = expand_str(
                "",
                &format!("pub trait G: Send + Sync {{ fn f(&self, v: {spelled}) -> Result<std::string::String, ::guatiao::library::ProviderError>; }}"),
            )
            .expect("expands");
            assert!(out.contains(shape), "{spelled}: {out}");
            assert!(out.contains("text_ret"), "{spelled}: {out}");
        }
        // Under a `crate =` root too.
        let out = expand_str(
            "crate = ::kinds::guatiao",
            "pub trait G: Send + Sync { fn f(&self, v: &kinds::guatiao::Value) -> Result<kinds::guatiao::Map, kinds::guatiao::library::ProviderError>; }",
        )
        .expect("expands");
        assert!(out.contains("value_arg"), "{out}");
        assert!(out.contains("map_ret"), "{out}");
        // A type of the caller's that merely ends in `Text` crosses by value.
        let out = expand_str(
            "",
            "pub trait G: Send + Sync { fn f(&self, t: proto::Text) -> Result<(), ProviderError>; }",
        )
        .expect("expands");
        assert!(
            out.contains("< proto :: Text as :: guatiao :: FromValue >"),
            "{out}"
        );
    }

    #[test]
    fn the_name_defaults_to_snake_case_and_can_be_given() {
        assert_eq!(snake_case("Greeter"), "greeter");
        assert_eq!(snake_case("SessionBackend"), "session_backend");
        assert_eq!(snake_case("HTTPServer"), "http_server");
        assert_eq!(snake_case("Codec2"), "codec2");
        let out = expand_str(
            "name = \"hello\"",
            "pub trait Greeter: Send + Sync { fn greet(&self) -> i64; }",
        )
        .unwrap();
        assert!(out.contains("const NAME : & 'static str = \"hello\""));
        let out = expand_str(
            "",
            "pub trait SessionBackend: Send + Sync { fn open(&self) -> i64; }",
        )
        .unwrap();
        assert!(out.contains("\"session_backend\""));
    }

    /// Every refusal, pinned by text so a message cannot drift into
    /// something that points into the expansion.
    #[test]
    fn kind_rejects_by_name() {
        let cases: &[(&str, &str, &str)] = &[
            (
                "",
                "pub trait G { fn f(&self) -> i64; }",
                "a kind must name `Send + Sync` as supertraits",
            ),
            (
                "",
                "pub trait G<T>: Send + Sync { fn f(&self) -> i64; }",
                "a kind cannot be generic",
            ),
            (
                "",
                "pub unsafe trait G: Send + Sync { fn f(&self) -> i64; }",
                "a kind cannot be an `unsafe trait`",
            ),
            (
                "",
                "pub trait G: Send + Sync {}",
                "a kind needs at least one method",
            ),
            (
                "",
                "pub trait G: Send + Sync { const N: usize; fn f(&self) -> i64; }",
                "a kind cannot declare an associated const",
            ),
            (
                "",
                "pub trait G: Send + Sync { type T; fn f(&self) -> i64; }",
                "a kind cannot declare an associated type",
            ),
            (
                "",
                "pub trait G: Send + Sync { async fn f(&self) -> i64; }",
                "an `async fn` cannot cross a C boundary",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f<T>(&self) -> i64; }",
                "a kind method cannot be generic",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&mut self) -> i64; }",
                "a kind method takes `&self`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(self) -> i64; }",
                "a kind method takes `&self`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f() -> i64; }",
                "an associated function has no provider to call",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> &str; }",
                "a borrowed return cannot cross",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> impl Iterator<Item = i64>; }",
                "an `impl Trait` return cannot cross",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: impl Fn()) -> i64; }",
                "an `impl Trait` argument cannot cross",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: &mut String) -> i64; }",
                "a `&mut` argument of this type cannot cross",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: &mut Text) -> i64; }",
                "`&mut Map`, `&mut List` and `&mut Value` are lent for the call",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: Option<&mut Map>) -> i64; }",
                "an `Option<&mut T>` argument cannot cross",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: Option<&mut Value>) -> i64; }",
                "lend `&mut Value`, which can hold null",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, m: &mut foreign::Map) -> i64; }",
                "this type ends in `Map`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: &Vec<u8>) -> i64; }",
                "a borrowed argument of this type cannot cross",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: Text) -> i64; }",
                "a `Text` cannot cross by value",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: Buffer) -> i64; }",
                "a `Buffer` cannot cross by value",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> Result<i64, String>; }",
                "the error of a kind method is `ProviderError`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: Config) -> i64; }",
                "must return `Result<_, ProviderError>`, because the conversion or the validation can fail",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> Config; }",
                "must return `Result<_, ProviderError>`, because the conversion or the validation can fail",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> Object<dyn K>; }",
                "or passing an `Object`, must return `Result<_, ProviderError>`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, dst: &mut [u8]) -> i64; }",
                "`&mut [u8]` crosses as an out-buffer, on an object kind only",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, l: &mut dyn Listener) -> i64; }",
                "so must one lending an object",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, l: &mut (dyn Listener + Send)) -> Result<(), ProviderError>; }",
                "naming the kind's trait alone",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> &mut dyn Listener; }",
                "a borrowed return cannot cross",
            ),
            (
                "object",
                "pub trait G: Send { fn f(&mut self, dst: &mut Vec<u8>) -> i64; }",
                "a `&mut` argument of this type cannot cross",
            ),
            (
                "object",
                "pub trait G { fn f(&mut self) -> i64; }",
                "an object kind must name `Send` as a supertrait",
            ),
            (
                "object = 1",
                "pub trait G: Send { fn f(&mut self) -> i64; }",
                "`object` takes no value",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn a(&self) -> i64 { 0 } fn b(&self) -> i64; }",
                "a required method cannot follow one with a default body",
            ),
            (
                "nam = \"x\"",
                "pub trait G: Send + Sync { fn f(&self) -> i64; }",
                "#[kind] takes only `name = \"...\"`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn header(&self) -> i64; }",
                "a kind method cannot be named `header`",
            ),
            (
                "object",
                "pub trait G: Send { fn destroy(&mut self) -> i64; }",
                "a kind method cannot be named `destroy`",
            ),
            (
                "object",
                "pub trait G: Send { fn into_object(&self) -> i64; }",
                "a kind method cannot be named `into_object`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, __out: i64) -> i64; }",
                "cannot start with `__`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, v: &foreign::Value) -> Result<i64, ProviderError>; }",
                "this type ends in `Value` under a path the kind attribute does not read",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, v: Option<&foreign::Value>) -> i64; }",
                "guatiao's `Value` is spelled `Value`, or under `guatiao::`, `::guatiao::` or the `crate =` root",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, m: &foreign::Map) -> Result<foreign::Map, ProviderError>; }",
                "this type ends in `Map`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> Result<foreign::String, ProviderError>; }",
                "the standard `String` is spelled `String` or under `std::`, `core::` or `alloc::`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> Result<model::List, ProviderError>; }",
                "this type ends in `List`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, o: other::Object<dyn K>) -> Result<(), ProviderError>; }",
                "this type ends in `Object`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> Result<i64, mine::ProviderError>; }",
                "this type ends in `ProviderError`",
            ),
            (
                "name = \"\"",
                "pub trait G: Send + Sync { fn f(&self) -> i64; }",
                "a kind's name cannot be empty",
            ),
        ];
        for (attr, item, expected) in cases {
            let err = expand_str(attr, item).expect_err(&format!("{item} must be refused"));
            assert!(
                err.contains(expected),
                "for `{item}` expected a message containing `{expected}`, got `{err}`"
            );
        }
    }
}
