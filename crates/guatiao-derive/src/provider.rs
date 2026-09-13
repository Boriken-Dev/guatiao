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
//! - `config = T` declares the configuration schema through `T: Schema`.
//! - `new = path` is a `fn() -> Self`; `new_with_host = path` is a
//!   `fn(Host) -> Self`; neither means `Default`.
//! - `available = path` is a `fn(&Self) -> Result<(), &'static str>`,
//!   asked on every call.
//!
//! What is emitted: one `static` table per kind (`<Kind>Vtable::of::<T>()`),
//! a `OnceLock<T>` holding the instance, an `impl ProviderDecl for T`
//! building a `ProviderParts`, and a compile-time check that `T`
//! implements every kind named — so a missing `impl` reads as an ordinary
//! trait error on the type's own span.

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
    Ok(emit(&ast.ident, &decl))
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

fn emit(ty: &Ident, decl: &Decl) -> TokenStream {
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
    let config = match &decl.config {
        Some(config) => quote! {
            ::core::option::Option::Some(<#config as ::guatiao::Schema>::schema(alloc)?)
        },
        None => quote! { ::core::option::Option::None },
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

            impl ::guatiao::library::kind::ProviderDecl for #ty {
                fn provider(
                    host: ::guatiao::library::Host,
                    alloc: ::guatiao::Alloc,
                ) -> ::core::result::Result<::guatiao::library::kind::ProviderParts, ::guatiao::ValueError> {
                    let instance: &'static #ty = __GUATIAO_INSTANCE.get_or_init(|| #build);
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
                        instance as *const #ty as *mut ::core::ffi::c_void,
                        #available_slot,
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
            out.contains(":: core :: option :: Option :: None ,)"),
            "no available slot"
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
