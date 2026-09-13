// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[guatiao::kind]`: a provider kind declared as a trait.
//!
//! The trait is emitted unchanged. Beside it:
//!
//! - `<Trait>Vtable`, a `repr(C)` table: a `KindHeader`, then one slot per
//!   method in declaration order, each a monomorphised shim;
//! - `<Trait>Vtable::of::<T>()`, a `const fn` filling the table for an
//!   implementation, and one `<method>_end()` per slot;
//! - `impl Kind for dyn Trait`, naming the table, its floor and its hash;
//! - `impl Trait for Remote<dyn Trait>`, the proxy that calls through a
//!   table it was handed;
//! - `From<Remote<dyn Trait>>` for `Box<dyn Trait>` (`Box` is the one
//!   fundamental type the orphan rule allows; `Kind::shared` makes an `Arc`).
//!
//! Every `unsafe` the shims and the proxy need is a call into
//! `::guatiao::library::kind`; this file writes tokens and never runs any.
//!
//! # What may cross
//!
//! A method takes `&self` and the trait names `Send + Sync`. Arguments:
//! integers, floats, `bool`, `&str`, `&[u8]`, `&Value`, `Option<&Value>`,
//! `&Map`, and any other type by value, which crosses as a value through
//! `ToValue`. Returns: `()`, the scalars, `Value`, `Map`, `List`, `String`,
//! and any other type through `FromValue` — each optionally inside
//! `Result<_, ProviderError>`. A method converting through `ToValue` or
//! `FromValue` must return a `Result`, because the conversion can fail.
//!
//! # Versioning
//!
//! Slots follow declaration order. A method **with a default body** is an
//! appended slot: a table too short to hold it makes the proxy run the
//! default. A required method after a defaulted one is refused, so the
//! floor is one contiguous prefix. The header's hash covers the required
//! methods' names and signatures, so a table built from a different
//! declaration is refused rather than called with the wrong ABI.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Block, FnArg, GenericArgument, Ident, ItemTrait, PathArguments, ReturnType, TraitItem,
    TraitItemFn, Type, TypeReference,
};

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
    let name = parse_name(attr, &tr.ident)?;
    check_trait(&tr)?;
    let methods = plan_methods(&tr)?;
    Ok(emit(&tr, &name, &methods))
}

// --- what the attribute takes ---------------------------------------------

/// `#[kind]` or `#[kind(name = "...")]`. The name defaults to the trait's
/// ident in snake case.
fn parse_name(attr: TokenStream, ident: &Ident) -> syn::Result<String> {
    let mut name = None;
    if !attr.is_empty() {
        let parser = syn::meta::parser(|meta| {
            if meta.path.is_ident("name") {
                let lit: syn::LitStr = meta.value()?.parse()?;
                if lit.value().is_empty() {
                    return Err(meta.error("a kind's name cannot be empty"));
                }
                name = Some(lit.value());
                Ok(())
            } else {
                Err(meta.error("#[kind] takes only `name = \"...\"`"))
            }
        });
        syn::parse::Parser::parse2(parser, attr)?;
    }
    Ok(name.unwrap_or_else(|| snake_case(&ident.to_string())))
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

fn check_trait(tr: &ItemTrait) -> syn::Result<()> {
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
    if !(names_send && names_sync) {
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
    /// Any other type by value, built into a value with `ToValue` and
    /// read back with `FromValue`.
    Owned(Type),
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
    default: Option<Block>,
    /// The normalised signature the hash covers.
    signature: String,
}

fn plan_methods(tr: &ItemTrait) -> syn::Result<Vec<MethodPlan>> {
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
        let plan = plan_method(f)?;
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
    Ok(plans)
}

fn plan_method(f: &TraitItemFn) -> syn::Result<MethodPlan> {
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
    match sig.receiver() {
        Some(r) if r.reference.is_some() && r.mutability.is_none() => {}
        Some(r) => {
            return Err(syn::Error::new_spanned(
                r,
                "a kind method takes `&self`: a provider is shared between callers, so `&mut \
                 self` and `self` cannot cross",
            ));
        }
        None => {
            return Err(syn::Error::new_spanned(
                &sig.ident,
                "a kind method takes `&self`; an associated function has no provider to call",
            ));
        }
    }

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
        let kind = arg_kind(&pat.ty)?;
        args.push(ArgPlan {
            ident,
            kind,
            ty: (*pat.ty).clone(),
        });
    }

    let (ret, fallible) = ret_kind(&sig.output)?;
    let converts = args.iter().any(|a| matches!(a.kind, ArgKind::Owned(_)))
        || matches!(ret, RetKind::Owned(_));
    if converts && !fallible {
        return Err(syn::Error::new_spanned(
            &sig.ident,
            "a method converting an argument or its return through `ToValue`/`FromValue` must \
             return `Result<_, ProviderError>`, because the conversion can fail",
        ));
    }

    let signature = normalised_signature(sig);
    Ok(MethodPlan {
        ident: sig.ident.clone(),
        args,
        ret,
        ret_ty: sig.output.clone(),
        fallible,
        default: f.default.clone(),
        signature,
    })
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

fn arg_kind(ty: &Type) -> syn::Result<ArgKind> {
    match ty {
        Type::Reference(TypeReference {
            mutability: Some(m),
            ..
        }) => Err(syn::Error::new_spanned(
            m,
            "a `&mut` argument cannot cross: the callee would write into memory the caller \
             still describes",
        )),
        Type::Reference(TypeReference { elem, .. }) => match &**elem {
            Type::Path(_) if last_ident(elem).is_some_and(|i| i == "str") => Ok(ArgKind::Str),
            Type::Slice(s) if last_ident(&s.elem).is_some_and(|i| i == "u8") => Ok(ArgKind::Bytes),
            Type::Path(_) if last_ident(elem).is_some_and(|i| i == "Value") => {
                Ok(ArgKind::ValueRef)
            }
            Type::Path(_) if last_ident(elem).is_some_and(|i| i == "Map") => Ok(ArgKind::MapRef),
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
            if last.ident == "Option"
                && let PathArguments::AngleBracketed(a) = &last.arguments
                && let Some(GenericArgument::Type(Type::Reference(r))) = a.args.first()
                && last_ident(&r.elem).is_some_and(|i| i == "Value")
            {
                return Ok(ArgKind::ValueOpt);
            }
            if matches!(
                last.ident.to_string().as_str(),
                "Value" | "Map" | "List" | "Text" | "Buffer"
            ) {
                return Err(syn::Error::new_spanned(
                    ty,
                    "take a value argument by reference (`&Value`, `&Map`); by value it would \
                     have to be copied across",
                ));
            }
            Ok(ArgKind::Owned(ty.clone()))
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "an argument of this shape cannot cross a C boundary",
        )),
    }
}

fn ret_kind(output: &ReturnType) -> syn::Result<(RetKind, bool)> {
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
        if last_ident(err).is_none_or(|i| i != "ProviderError") {
            return Err(syn::Error::new_spanned(
                err,
                "the error of a kind method is `ProviderError`, the one error type both sides \
                 of the table share",
            ));
        }
        return Ok((plain_ret(ok)?, true));
    }
    Ok((plain_ret(ty)?, false))
}

fn plain_ret(ty: &Type) -> syn::Result<RetKind> {
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
            Ok(match last.ident.to_string().as_str() {
                "Value" => RetKind::Value,
                "Map" => RetKind::Map,
                "List" => RetKind::List,
                "String" => RetKind::Text,
                _ => RetKind::Owned(ty.clone()),
            })
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "a return of this shape cannot cross a C boundary",
        )),
    }
}

/// `name(ty,ty)->ret` with every space removed, so a re-typed or
/// reordered required slot changes the hash and a reformatted one does
/// not.
fn normalised_signature(sig: &syn::Signature) -> String {
    let args: Vec<String> = sig
        .inputs
        .iter()
        .skip(1)
        .filter_map(|a| match a {
            FnArg::Typed(t) => {
                let ty = &t.ty;
                Some(strip(&quote!(#ty)))
            }
            FnArg::Receiver(_) => None,
        })
        .collect();
    let ret = match &sig.output {
        ReturnType::Default => "()".to_string(),
        ReturnType::Type(_, ty) => strip(&quote!(#ty)),
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

fn emit(tr: &ItemTrait, name: &str, methods: &[MethodPlan]) -> TokenStream {
    let trait_ident = &tr.ident;
    let vis = &tr.vis;
    let table = format_ident!("{}Vtable", trait_ident);

    let required: Vec<&MethodPlan> = methods.iter().filter(|m| m.default.is_none()).collect();
    let hash_input = required
        .iter()
        .map(|m| m.signature.clone())
        .collect::<Vec<_>>()
        .join(";");
    let last_required = required.last().expect("at least one required method");
    let floor_end = end_ident(&last_required.ident);

    let slot_fields = methods.iter().map(|m| {
        let ident = &m.ident;
        let ty = slot_type(m);
        quote! { pub #ident: ::core::option::Option<#ty>, }
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
    let shims = methods.iter().map(|m| emit_shim(trait_ident, m));
    let required_slots = required.iter().map(|m| {
        let name = m.ident.to_string();
        let end = end_ident(&m.ident);
        quote! { (#name, #table::#end()), }
    });
    let proxies = methods.iter().map(|m| emit_proxy(&table, m));

    quote! {
        #tr

        #[repr(C)]
        #[doc(hidden)]
        #[allow(non_snake_case)]
        #vis struct #table {
            pub header: ::guatiao::library::KindHeader,
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
                    #(#slot_inits)*
                }
            }

            /// One past the last required slot. Frozen.
            #[doc(hidden)]
            pub const fn floor() -> usize {
                Self::#floor_end()
            }

            #(#end_fns)*

            #(#shims)*
        }

        impl ::guatiao::library::Kind for dyn #trait_ident {
            const NAME: &'static str = #name;
            type Vtable = #table;
            const FLOOR: usize = #table::floor();
            const FLOOR_HASH: u32 = ::guatiao::library::kind::fnv1a(#hash_input);
            const REQUIRED: &'static [(&'static str, usize)] = &[ #(#required_slots)* ];

            fn as_dyn(remote: &::guatiao::library::Remote<Self>) -> &Self {
                remote
            }
            fn boxed(remote: ::guatiao::library::Remote<Self>) -> ::std::boxed::Box<Self> {
                ::std::boxed::Box::new(remote)
            }
            fn shared(remote: ::guatiao::library::Remote<Self>) -> ::std::sync::Arc<Self> {
                ::std::sync::Arc::new(remote)
            }
        }

        #[allow(clippy::needless_question_mark)]
        impl #trait_ident for ::guatiao::library::Remote<dyn #trait_ident> {
            #(#proxies)*
        }

        impl ::core::convert::From<::guatiao::library::Remote<dyn #trait_ident>>
            for ::std::boxed::Box<dyn #trait_ident>
        {
            fn from(remote: ::guatiao::library::Remote<dyn #trait_ident>) -> Self {
                ::std::boxed::Box::new(remote)
            }
        }
    }
}

fn end_ident(method: &Ident) -> Ident {
    format_ident!("{}_end", method)
}

fn shim_ident(method: &Ident) -> Ident {
    format_ident!("__guatiao_shim_{}", method)
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
    }
}

/// The slot's function-pointer type.
fn slot_type(m: &MethodPlan) -> TokenStream {
    let args = m.args.iter().map(|a| {
        let ident = &a.ident;
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

/// The shim: reads its arguments, calls the implementation, writes the
/// answer. Every `unsafe` is a call into the runtime.
fn emit_shim(trait_ident: &Ident, m: &MethodPlan) -> TokenStream {
    let shim = shim_ident(&m.ident);
    let method = &m.ident;
    let params = m.args.iter().map(|a| {
        let ident = &a.ident;
        let ty = arg_c_type(&a.kind);
        quote!(#ident: #ty)
    });
    let out_param = ret_c_type(&m.ret).map(|ty| quote!(, out: #ty));
    let err_param = m
        .fallible
        .then(|| quote!(, err: *mut ::guatiao::library::ProviderError));

    // A conversion failure inside a fallible method is an error the
    // caller sees; the plan refuses conversions in infallible ones.
    let fail = |status: TokenStream| {
        if m.fallible {
            quote! {
                // SAFETY: the proxy passes a writable error slot.
                return unsafe { ::guatiao::library::kind::write_err(err, #status.into()) };
            }
        } else {
            quote! { return #status; }
        }
    };

    let reads = m.args.iter().map(|a| {
        let ident = &a.ident;
        match &a.kind {
            ArgKind::Scalar(_) => quote!(),
            ArgKind::Str => {
                let f = fail(quote!(__s));
                quote! {
                    // SAFETY: the proxy passes a readable view.
                    let #ident = match unsafe { ::guatiao::library::kind::str_arg(#ident) } {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__s) => { #f }
                    };
                }
            }
            ArgKind::Bytes => {
                let f = fail(quote!(__s));
                quote! {
                    // SAFETY: the proxy passes a readable view.
                    let #ident = match unsafe { ::guatiao::library::kind::bytes_arg(#ident) } {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__s) => { #f }
                    };
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
        }
    });

    let arg_idents = m.args.iter().map(|a| &a.ident);
    let call = quote!(__this.#method(#(#arg_idents),*));

    // What the implementation answered, as the status the shim returns.
    let write = |answer: TokenStream| match &m.ret {
        RetKind::Unit => quote! { { let _ = #answer; ::guatiao::Status::GUATIAO_OK } },
        RetKind::Scalar(_) | RetKind::Value | RetKind::Map | RetKind::List => quote! {
            // SAFETY: the proxy passes a writable out slot.
            unsafe { ::guatiao::library::kind::write_out(out, #answer) }
        },
        RetKind::Text => quote! {
            // SAFETY: the proxy passes a writable out slot.
            unsafe { ::guatiao::library::kind::write_out(out, ::guatiao::Text::new(&#answer)) }
        },
        RetKind::Owned(ty) => quote! {
            match <#ty as ::guatiao::ToValue>::to_value(&#answer, ::guatiao::Alloc::rust()) {
                // SAFETY: the proxy passes a writable out slot.
                ::core::result::Result::Ok(__v) => unsafe { ::guatiao::library::kind::write_out(out, __v) },
                // SAFETY: the proxy passes a writable error slot.
                ::core::result::Result::Err(__e) => unsafe {
                    ::guatiao::library::kind::write_err(err, ::guatiao::library::ProviderError::from(__e))
                },
            }
        },
    };
    let body = if m.fallible {
        let ok = write(quote!(__answer));
        quote! {
            match #call {
                ::core::result::Result::Ok(__answer) => { #ok }
                // SAFETY: the proxy passes a writable error slot.
                ::core::result::Result::Err(__e) => unsafe { ::guatiao::library::kind::write_err(err, __e) },
            }
        }
    } else {
        write(call)
    };

    quote! {
        /// # Safety
        ///
        /// Called through the table `of::<__T>()` built, with the `ctx`
        /// that table's provider declared: a `&__T`.
        #[doc(hidden)]
        pub unsafe extern "C" fn #shim<__T: #trait_ident>(
            ctx: *mut ::core::ffi::c_void
            #(, #params)*
            #out_param
            #err_param
        ) -> ::guatiao::Status {
            ::guatiao::library::kind::catch(|| {
                // SAFETY: the table was built for `__T` and the descriptor's
                // `ctx` is a `&__T`.
                let ::core::option::Option::Some(__this) =
                    (unsafe { ::guatiao::library::kind::ctx_ref::<__T>(ctx) })
                else {
                    return ::guatiao::Status::GUATIAO_ERR_NULL;
                };
                #(#reads)*
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
    let params = m.args.iter().map(|a| {
        let ident = &a.ident;
        let ty = &a.ty;
        quote!(#ident: #ty)
    });

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

    let marshals = m.args.iter().map(|a| {
        let ident = &a.ident;
        let name = format_ident!("__arg_{}", ident);
        match &a.kind {
            ArgKind::Scalar(_) => quote! { let #name = #ident; },
            ArgKind::Str => quote! { let #name = ::guatiao::Str::borrowed(#ident); },
            ArgKind::Bytes => quote! {
                let #name = ::guatiao::value::types::Bytes { ptr: #ident.as_ptr(), len: #ident.len() };
            },
            ArgKind::ValueRef => quote! { let #name = #ident as *const ::guatiao::Value; },
            ArgKind::MapRef => quote! { let #name = #ident as *const ::guatiao::Map; },
            ArgKind::ValueOpt => quote! {
                let #name = #ident.map_or(::core::ptr::null(), |__v| __v as *const ::guatiao::Value);
            },
            ArgKind::Owned(ty) => {
                let f = fail(quote!(::guatiao::library::ProviderError::from(__e)));
                quote! {
                    let #name = match <#ty as ::guatiao::ToValue>::to_value(&#ident, ::guatiao::Alloc::rust()) {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__e) => { #f }
                    };
                    let #name = &#name as *const ::guatiao::Value;
                }
            }
        }
    });
    let arg_names = m.args.iter().map(|a| format_ident!("__arg_{}", a.ident));

    let out_init = match &m.ret {
        RetKind::Unit => quote!(),
        RetKind::Scalar(ty) => quote! { let mut __out: #ty = ::core::default::Default::default(); },
        RetKind::Value | RetKind::Owned(_) => {
            quote! { let mut __out = ::guatiao::Value::absent(); }
        }
        RetKind::Map => quote! { let mut __out = ::guatiao::Map::new(); },
        RetKind::List => quote! { let mut __out = ::guatiao::List::new(); },
        RetKind::Text => quote! { let mut __out = ::guatiao::Text::new(""); },
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
        RetKind::Unit => quote!(()),
        RetKind::Scalar(_) | RetKind::Value | RetKind::Map | RetKind::List => quote!(__out),
        RetKind::Text => quote!(::std::string::ToString::to_string(
            __out.as_str().unwrap_or("")
        )),
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
    };
    let answer = if m.fallible {
        quote!(::core::result::Result::Ok(#answer))
    } else {
        answer
    };

    let call = quote! {
        #(#marshals)*
        #out_init
        #err_init
        // SAFETY: the slot's own signature, read under the table's size;
        // every pointer is a live local or a borrowed argument.
        let __status = unsafe { __f(self.ctx() #(, #arg_names)* #out_arg #err_arg) };
        #on_status
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
        // body runs, exactly as it would on a type implementing the trait.
        Some(block) => quote! {
            // SAFETY: the slot at that offset has this type in `#table`.
            match unsafe { self.slot::<#slot_ty>(::core::mem::offset_of!(#table, #method), #table::#end()) } {
                ::core::option::Option::Some(__f) => { #call }
                ::core::option::Option::None => #block
            }
        },
    };

    quote! {
        fn #method(&self #(, #params)*) #ret_ty {
            #body
        }
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
                "impl Greeter",
                "impl :: core :: convert :: From < :: guatiao :: library :: Remote < dyn Greeter > >",
            ]
        );
        assert!(out.contains("const NAME : & 'static str = \"greeter\""));
        assert!(
            out.contains("fnv1a (\"greet(&str)->Result<String,ProviderError>\")"),
            "only the required method is hashed, normalised: {out}"
        );
        assert!(out.contains("(\"greet\" , GreeterVtable :: greet_end ())"));
    }

    #[test]
    fn the_expansion_matches_the_snapshot() {
        let out = expand_str("", GREETER).expect("expands");
        let file: syn::File = syn::parse_str(&out).unwrap();
        let pretty = prettyplease::unparse(&file);
        let expected = include_str!("snapshots/greeter.expected.rs");
        if pretty != expected {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/snapshots/greeter.actual.rs"
            );
            std::fs::write(path, &pretty).unwrap();
            panic!(
                "the expansion changed; compare src/snapshots/greeter.actual.rs with greeter.expected.rs and move it over if the change is intended"
            );
        }
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
                "pub trait G: Send + Sync { fn f(&self, x: &mut Value) -> i64; }",
                "a `&mut` argument cannot cross",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: &Vec<u8>) -> i64; }",
                "a borrowed argument of this type cannot cross",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: Value) -> i64; }",
                "take a value argument by reference",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> Result<i64, String>; }",
                "the error of a kind method is `ProviderError`",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self, x: Config) -> i64; }",
                "must return `Result<_, ProviderError>`, because the conversion can fail",
            ),
            (
                "",
                "pub trait G: Send + Sync { fn f(&self) -> Config; }",
                "must return `Result<_, ProviderError>`, because the conversion can fail",
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
