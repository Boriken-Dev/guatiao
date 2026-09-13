// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[derive(Provider)]`: a type implementing kind traits becomes a
//! provider, with every piece of glue generated.
//!
//! ```ignore
//! #[derive(Default, Provider)]
//! #[provider(Greeter, Counter)]
//! struct Hello;
//! ```
//!
//! The attribute names the kinds; everything else defaults. The long form:
//!
//! ```ignore
//! #[provider(kinds(Greeter), id = "acme_hello", name = "Hello", version = "1.0.0",
//!            config = HelloConfig, new = Hello::build, available = Hello::ready)]
//! ```
//!
//! - `id` defaults to `{CARGO_PKG_NAME}_{type in snake case}`, with `-` in
//!   the package name as `_`; `name` to the type's ident; `version` to
//!   empty, which means the library's.
//! - `config = C` makes the provider **built from `C`**: the schema comes
//!   from `C: Schema`, a configuration value is decoded through `C:
//!   FromValue`, and an instance is `Self: TryFrom<C, Error:
//!   Into<ProviderError>>` (`C = Self` is the identity `From`). The
//!   `create`/`destroy` slots are emitted; each instance's address is the
//!   `ctx` the kind tables call with.
//! - `new = path` is a `fn() -> Self`; `new_with_host = path` is a
//!   `fn(Host) -> Self`; neither means `Default` — except with `config`,
//!   where neither means no default instance at all.
//! - `available = path` is a `fn(&Self) -> Result<(), &'static str>`,
//!   asked on every call of the default instance; refused without one.
//!
//! What is emitted: one `static` table per kind (`<Kind>Vtable::of::<T>()`),
//! a `OnceLock<T>` holding the default instance, the `create`/`destroy`
//! shims when `config` is given, an `impl ProviderDecl for T` building a
//! `ProviderParts`, and a compile-time check that `T` implements every
//! kind named — so a missing `impl` reads as an ordinary trait error on
//! the type's own span.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Ident, Path, Type};

use crate::kind::snake_case;

/// Expands the derive, or emits a compile error at the user's span.
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    match try_expand(input) {
        Ok(tokens) => tokens,
        Err(e) => e.to_compile_error(),
    }
}

#[derive(Default)]
struct Decl {
    kinds: Vec<Path>,
    id: Option<String>,
    name: Option<String>,
    version: Option<String>,
    config: Option<Type>,
    new: Option<Path>,
    new_with_host: Option<Path>,
    available: Option<Path>,
}

fn try_expand(input: TokenStream) -> syn::Result<TokenStream> {
    let ast: DeriveInput = syn::parse2(input)?;
    if ast.generics.params.iter().next().is_some() {
        return Err(syn::Error::new_spanned(
            &ast.generics,
            "a provider cannot be generic: one type is one instance behind one descriptor",
        ));
    }
    let decl = read_attrs(&ast)?;
    if decl.kinds.is_empty() {
        return Err(syn::Error::new_spanned(
            &ast.ident,
            "a provider names at least one kind: `#[provider(Greeter)]`",
        ));
    }
    if decl.new.is_some() && decl.new_with_host.is_some() {
        return Err(syn::Error::new_spanned(
            &ast.ident,
            "`new` and `new_with_host` are two ways to build the one instance; name one",
        ));
    }
    let has_default_instance =
        decl.config.is_none() || decl.new.is_some() || decl.new_with_host.is_some();
    if decl.available.is_some() && !has_default_instance {
        return Err(syn::Error::new_spanned(
            &ast.ident,
            "`available` asks the provider's default instance, and a provider built only from a \
             configuration has none; add `new` or `new_with_host`, or drop `available`",
        ));
    }
    Ok(emit(&ast.ident, &decl, has_default_instance))
}

fn read_attrs(ast: &DeriveInput) -> syn::Result<Decl> {
    let mut decl = Decl::default();
    let mut seen = false;
    for attr in &ast.attrs {
        if !attr.path().is_ident("provider") {
            continue;
        }
        seen = true;
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            match key.as_str() {
                "kinds" => meta.parse_nested_meta(|kind| {
                    decl.kinds.push(kind.path.clone());
                    Ok(())
                }),
                "id" => set_str(&meta, &mut decl.id, "id"),
                "name" => set_str(&meta, &mut decl.name, "name"),
                "version" => set_str(&meta, &mut decl.version, "version"),
                "config" => {
                    once(&meta, decl.config.is_some(), "config")?;
                    decl.config = Some(meta.value()?.parse()?);
                    Ok(())
                }
                "new" => {
                    once(&meta, decl.new.is_some(), "new")?;
                    decl.new = Some(meta.value()?.parse()?);
                    Ok(())
                }
                "new_with_host" => {
                    once(&meta, decl.new_with_host.is_some(), "new_with_host")?;
                    decl.new_with_host = Some(meta.value()?.parse()?);
                    Ok(())
                }
                "available" => {
                    once(&meta, decl.available.is_some(), "available")?;
                    decl.available = Some(meta.value()?.parse()?);
                    Ok(())
                }
                // A bare path with no `=` is a kind: `#[provider(Greeter)]`.
                _ if !meta.input.peek(syn::Token![=]) => {
                    decl.kinds.push(meta.path.clone());
                    Ok(())
                }
                other => Err(meta.error(format!(
                    "`{other}` is not a provider option; the options are `kinds(..)`, `id`, \
                     `name`, `version`, `config`, `new`, `new_with_host` and `available`"
                ))),
            }
        })?;
    }
    if !seen {
        return Err(syn::Error::new_spanned(
            &ast.ident,
            "a provider needs `#[provider(..)]` naming its kinds",
        ));
    }
    Ok(decl)
}

fn once(meta: &syn::meta::ParseNestedMeta<'_>, already: bool, what: &str) -> syn::Result<()> {
    if already {
        return Err(meta.error(format!("`{what}` is given twice")));
    }
    Ok(())
}

fn set_str(
    meta: &syn::meta::ParseNestedMeta<'_>,
    slot: &mut Option<String>,
    what: &str,
) -> syn::Result<()> {
    once(meta, slot.is_some(), what)?;
    let lit: syn::LitStr = meta.value()?.parse()?;
    *slot = Some(lit.value());
    Ok(())
}

/// The kind's table type, reached through the trait so only the trait
/// has to be in scope: `<dyn Greeter as Kind>::Vtable`.
fn vtable_type(kind: &Path) -> TokenStream {
    quote! { <dyn #kind as ::guatiao::library::Kind>::Vtable }
}

fn emit(ty: &Ident, decl: &Decl, has_default_instance: bool) -> TokenStream {
    let snake = snake_case(&ty.to_string());
    let name = decl.name.clone().unwrap_or_else(|| ty.to_string());
    let version = decl.version.clone().unwrap_or_default();
    let id = match &decl.id {
        Some(id) => quote! { ::std::string::String::from(#id) },
        None => quote! {
            ::std::format!("{}_{}", env!("CARGO_PKG_NAME").replace('-', "_"), #snake)
        },
    };

    let checks = decl.kinds.iter().enumerate().map(|(i, kind)| {
        let check = format_ident!("__guatiao_implements_{}", i);
        quote! {
            fn #check<__T: #kind>() {}
            let _ = #check::<#ty> as fn();
        }
    });
    let tables = decl.kinds.iter().enumerate().map(|(i, kind)| {
        let table = format_ident!("__GUATIAO_TABLE_{}", i);
        let vtable = vtable_type(kind);
        quote! {
            static #table: #vtable = <#vtable>::of::<#ty>();
        }
    });
    let table_entries = decl.kinds.iter().enumerate().map(|(i, kind)| {
        let table = format_ident!("__GUATIAO_TABLE_{}", i);
        let vtable = vtable_type(kind);
        quote! {
            ::guatiao::library::KindTable::new(
                <dyn #kind as ::guatiao::library::Kind>::NAME,
                &#table as *const #vtable as *const ::core::ffi::c_void,
                ::core::mem::size_of::<#vtable>(),
            ),
        }
    });
    let kind_names = decl
        .kinds
        .iter()
        .map(|kind| quote! { <dyn #kind as ::guatiao::library::Kind>::NAME, });

    let build = match (&decl.new, &decl.new_with_host) {
        (Some(new), _) => quote! { #new() },
        (_, Some(new)) => quote! { #new(host) },
        _ => quote! { <#ty as ::core::default::Default>::default() },
    };
    // The default instance: the one `ctx` of the descriptor. A provider
    // built only from a configuration has none, and its `ctx` is null.
    let instance = if has_default_instance {
        quote! {
            let instance: *mut ::core::ffi::c_void =
                (__GUATIAO_INSTANCE.get_or_init(|| #build) as *const #ty as *mut #ty)
                    .cast::<::core::ffi::c_void>();
        }
    } else {
        quote! {
            let _ = host;
            let instance: *mut ::core::ffi::c_void = ::core::ptr::null_mut();
        }
    };
    let config = match &decl.config {
        Some(config) => quote! {
            ::core::option::Option::Some(<#config as ::guatiao::Schema>::schema(alloc)?)
        },
        None => quote! { ::core::option::Option::None },
    };
    // Instances from a configuration: decode it as `C`, build `Self`
    // through `TryFrom<C>`, box it. `C = Self` is the identity `From`.
    let (create_shims, create_slot, destroy_slot) = match &decl.config {
        Some(config) => (
            quote! {
                /// # Safety
                ///
                /// Called through the descriptor: `config` is null or a
                /// well-formed value, `out` and `err` are null or writable.
                unsafe extern "C" fn __guatiao_create(
                    _ctx: *mut ::core::ffi::c_void,
                    config: *const ::guatiao::Value,
                    out: *mut *mut ::core::ffi::c_void,
                    err: *mut ::guatiao::library::ProviderError,
                ) -> ::guatiao::Status {
                    ::guatiao::library::kind::catch(|| {
                        // SAFETY: the caller's contract.
                        let built = unsafe { ::guatiao::library::kind::config_arg::<#config>(config) }
                            .and_then(|__c| {
                                <#ty as ::core::convert::TryFrom<#config>>::try_from(__c)
                                    .map_err(::core::convert::Into::<::guatiao::library::ProviderError>::into)
                            });
                        // SAFETY: the caller's contract.
                        unsafe { ::guatiao::library::kind::instance_out::<#ty>(out, err, built) }
                    })
                }

                /// # Safety
                ///
                /// `instance` is null or came from `__guatiao_create`.
                unsafe extern "C" fn __guatiao_destroy(
                    _ctx: *mut ::core::ffi::c_void,
                    instance: *mut ::core::ffi::c_void,
                ) {
                    // SAFETY: the caller's contract.
                    unsafe { ::guatiao::library::kind::destroy_instance::<#ty>(instance) }
                }
            },
            quote! { ::core::option::Option::Some(__guatiao_create) },
            quote! { ::core::option::Option::Some(__guatiao_destroy) },
        ),
        None => (
            quote! {},
            quote! { ::core::option::Option::None },
            quote! { ::core::option::Option::None },
        ),
    };
    let (available_shim, available_slot) = match &decl.available {
        Some(ask) => (
            quote! {
                /// # Safety
                ///
                /// Called through the descriptor with the `ctx` it declared.
                unsafe extern "C" fn __guatiao_available(
                    ctx: *mut ::core::ffi::c_void,
                    reason: *mut ::guatiao::Str,
                ) -> bool {
                    // SAFETY: the descriptor's `ctx` is the instance below.
                    unsafe {
                        ::guatiao::library::kind::available_via::<#ty>(ctx, reason, |__this| #ask(__this))
                    }
                }
            },
            quote! { ::core::option::Option::Some(__guatiao_available) },
        ),
        None => (quote! {}, quote! { ::core::option::Option::None }),
    };

    quote! {
        const _: () = {
            #(#checks)*

            #(#tables)*

            /// The one instance, built on the first entry call.
            static __GUATIAO_INSTANCE: ::std::sync::OnceLock<#ty> = ::std::sync::OnceLock::new();

            #available_shim

            #create_shims

            impl ::guatiao::library::kind::ProviderDecl for #ty {
                fn provider(
                    host: ::guatiao::library::Host,
                    alloc: ::guatiao::Alloc,
                ) -> ::core::result::Result<::guatiao::library::kind::ProviderParts, ::guatiao::ValueError> {
                    #instance
                    let kinds: &[&'static str] = &[ #(#kind_names)* ];
                    let tables = ::std::vec![ #(#table_entries)* ];
                    let config = #config;
                    ::core::result::Result::Ok(::guatiao::library::kind::ProviderParts::new(
                        &#id,
                        #name,
                        #version,
                        kinds,
                        tables,
                        config,
                        instance,
                        #available_slot,
                        #create_slot,
                        #destroy_slot,
                    ))
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_str(item: &str) -> Result<String, String> {
        let item: TokenStream = item.parse().unwrap();
        match try_expand(item) {
            Ok(t) => Ok(t.to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    #[test]
    fn the_short_form_names_kinds_and_defaults_the_rest() {
        let out = expand_str("#[provider(Greeter, counter::Counter)] struct Hello;").unwrap();
        let file: syn::File = syn::parse_str(&out).expect("parses");
        assert_eq!(file.items.len(), 1, "one `const _` block");
        assert!(out.contains("static __GUATIAO_TABLE_0 : < dyn Greeter as :: guatiao :: library :: Kind > :: Vtable ="));
        assert!(out.contains(":: Vtable > :: of :: < Hello > ()"));
        assert!(out.contains("static __GUATIAO_TABLE_1 : < dyn counter :: Counter as"));
        assert!(out.contains("< Hello as :: core :: default :: Default > :: default ()"));
        assert!(out.contains("env ! (\"CARGO_PKG_NAME\") . replace ('-' , \"_\") , \"hello\""));
        assert!(
            out.contains("\"Hello\" , \"\" , kinds"),
            "name defaults to the ident, version to empty"
        );
        assert!(
            out.contains(
                ":: core :: option :: Option :: None , :: core :: option :: Option :: None , \
                 :: core :: option :: Option :: None ,)"
            ),
            "no available, create or destroy slot"
        );
        assert!(
            out.contains("__GUATIAO_INSTANCE . get_or_init"),
            "a default instance"
        );
    }

    #[test]
    fn a_configuration_builds_instances() {
        let out = expand_str("#[provider(Greeter, config = HiConfig)] struct Hi;").unwrap();
        assert!(out.contains("config_arg :: < HiConfig > (config)"));
        assert!(out.contains("< Hi as :: core :: convert :: TryFrom < HiConfig >> :: try_from"));
        assert!(out.contains(
            "Some (__guatiao_create) , :: core :: option :: Option :: Some (__guatiao_destroy) ,)"
        ));
        assert!(
            out.contains(":: core :: ptr :: null_mut ()") && !out.contains("get_or_init"),
            "no default instance without `new`: {out}"
        );
        let out = expand_str("#[provider(Greeter, config = HiConfig, new = Hi::make)] struct Hi;")
            .unwrap();
        assert!(
            out.contains("get_or_init"),
            "a default instance beside the built ones"
        );
    }

    #[test]
    fn the_long_form_overrides_everything() {
        let out = expand_str(
            "#[provider(kinds(Greeter), id = \"acme_hi\", name = \"Hi\", version = \"2.0.0\", \
             config = HiConfig, new_with_host = Hi::build, available = Hi::ready)] struct Hi;",
        )
        .unwrap();
        assert!(out.contains("String :: from (\"acme_hi\")"));
        assert!(out.contains("\"Hi\" , \"2.0.0\" , kinds"));
        assert!(out.contains("< HiConfig as :: guatiao :: Schema > :: schema (alloc) ?"));
        assert!(out.contains("Hi :: build (host)"));
        assert!(
            out.contains(
                "available_via :: < Hi > (ctx , reason , | __this | Hi :: ready (__this))"
            )
        );
        let out = expand_str("#[provider(Greeter, new = Hi::make)] struct Hi;").unwrap();
        assert!(out.contains("Hi :: make ()"));
    }

    #[test]
    fn provider_rejects_by_name() {
        let cases: &[(&str, &str)] = &[
            ("struct Hello;", "a provider needs `#[provider(..)]`"),
            (
                "#[provider()] struct Hello;",
                "a provider names at least one kind",
            ),
            (
                "#[provider(kinds())] struct Hello;",
                "expected nested attribute",
            ),
            (
                "#[provider(Greeter)] struct Hello<T>(T);",
                "a provider cannot be generic",
            ),
            (
                "#[provider(Greeter, new = A::a, new_with_host = A::b)] struct Hello;",
                "`new` and `new_with_host` are two ways to build the one instance",
            ),
            (
                "#[provider(Greeter, nam = \"x\")] struct Hello;",
                "`nam` is not a provider option",
            ),
            (
                "#[provider(Greeter, id = \"a\", id = \"b\")] struct Hello;",
                "`id` is given twice",
            ),
            (
                "#[provider(Greeter, id = 5)] struct Hello;",
                "expected string literal",
            ),
            (
                "#[provider(Greeter, config = C, available = Hello::ready)] struct Hello;",
                "`available` asks the provider's default instance",
            ),
        ];
        for (item, expected) in cases {
            let err = expand_str(item).expect_err(&format!("{item} must be refused"));
            assert!(
                err.contains(expected),
                "for `{item}` expected a message containing `{expected}`, got `{err}`"
            );
        }
    }
}
