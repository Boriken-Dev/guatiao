// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The streaming impls beside the tree ones: `#[derive(ToValue)]` also
//! writes `Walk`, `#[derive(FromValue)]` also `FromWalk`, so a derived
//! type goes to bytes or text and back with no tree in the middle.
//!
//! A struct with named fields, a tuple struct, a newtype and a unit
//! struct walk and read field by field. A field whose type has only the
//! tree traits is walked or read through its tree, chosen at compile time
//! (`guatiao::__private`'s `Field` and `Pick`), so every type that derived
//! before still does. An enum, and a struct with a flattened member, walk
//! and read through the tree their own impls build: which keys a variant
//! or a flattened member holds is known only once the whole map is.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Field, Fields, Ident, Index, Member, Type};

use crate::expand::{ContainerAttrs, Derive, FieldAttrs, FieldPlan};

/// `impl<..> #path for Name<..>`, every type parameter bound by `bound`.
fn head(
    path: TokenStream,
    bound: TokenStream,
    name: &Ident,
    generics: &syn::Generics,
) -> TokenStream {
    let mut generics = generics.clone();
    let params: Vec<Ident> = generics.type_params().map(|p| p.ident.clone()).collect();
    let clause = generics.make_where_clause();
    for param in params {
        clause.predicates.push(syn::parse_quote!(#param: #bound));
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        #[automatically_derived]
        impl #impl_generics #path for #name #ty_generics #where_clause
    }
}

fn walk_head(name: &Ident, generics: &syn::Generics) -> TokenStream {
    head(
        quote!(::guatiao::Walk),
        quote!(::guatiao::Walk),
        name,
        generics,
    )
}

fn from_walk_head(name: &Ident, generics: &syn::Generics) -> TokenStream {
    head(
        quote!(::guatiao::FromWalk),
        quote!(::guatiao::FromWalk),
        name,
        generics,
    )
}

/// The field, walking itself or walked as its tree. Clippy reads the
/// autoref as a needless borrow at the field's span, so every function
/// using it allows `clippy::needless_borrow`.
fn walkable(access: &TokenStream, ty: &Type) -> TokenStream {
    quote_spanned! {ty.span()=>
        &(&&::guatiao::__private::Field(#access)).__walkable()
    }
}

/// How a field of type `ty` is read: its own walk, or its tree.
fn picked(ty: &Type) -> TokenStream {
    quote_spanned! {ty.span()=>
        (&&<::guatiao::__private::Pick<#ty>>::new()).__seed()
    }
}

/// The streaming impl `derive` adds for a struct, or nothing for
/// `Schema`.
pub(crate) fn for_struct(
    derive: Derive,
    name: &Ident,
    generics: &syn::Generics,
    container: &ContainerAttrs,
    fields: &Fields,
) -> syn::Result<TokenStream> {
    if derive == Derive::Schema {
        return Ok(TokenStream::new());
    }
    match fields {
        Fields::Named(named) if container.transparent => {
            let mut kept = None;
            let mut skipped = Vec::new();
            for field in &named.named {
                if FieldAttrs::read(derive, field)?.skip {
                    skipped.push(field.ident.clone().expect("a named field"));
                } else {
                    kept = Some(field);
                }
            }
            let field = kept.expect("the tree impl refused a transparent struct with no field");
            let member = Member::Named(field.ident.clone().expect("a named field"));
            Ok(newtype(
                derive, name, generics, &member, &field.ty, &skipped,
            ))
        }
        Fields::Named(named) => {
            let mut plan = Vec::new();
            for field in &named.named {
                plan.push(FieldPlan::read(derive, field, container.rename_all)?);
            }
            if plan.iter().any(|f| f.flatten) {
                return Ok(through_tree(derive, name, generics));
            }
            Ok(match derive {
                Derive::ToValue => walk_map(name, generics, &plan),
                _ => read_map(name, generics, &plan, container.deny_unknown_fields),
            })
        }
        Fields::Unnamed(unnamed) => {
            let all: Vec<&Field> = unnamed.unnamed.iter().collect();
            if let [only] = all[..] {
                let member = Member::Unnamed(Index::from(0));
                return Ok(newtype(derive, name, generics, &member, &only.ty, &[]));
            }
            Ok(tuple(derive, name, generics, &all))
        }
        Fields::Unit => Ok(unit(derive, name, generics)),
    }
}

/// An enum's, or a flattening struct's, streaming impl: through the tree
/// its own `ToValue` builds and its own `FromValue` reads.
pub(crate) fn through_tree(derive: Derive, name: &Ident, generics: &syn::Generics) -> TokenStream {
    match derive {
        Derive::ToValue => {
            let head = head(
                quote!(::guatiao::Walk),
                quote!(::guatiao::ToValue),
                name,
                generics,
            );
            quote! {
                #head {
                    fn walk<__V: ::guatiao::value::visit::Visitor>(
                        &self,
                        __visitor: __V,
                    ) -> ::core::result::Result<__V::Ok, __V::Error> {
                        ::guatiao::__private::walk_tree(self, __visitor)
                    }
                }
            }
        }
        Derive::FromValue => {
            let head = head(
                quote!(::guatiao::FromWalk),
                quote!(::guatiao::FromValue),
                name,
                generics,
            );
            quote! {
                #head {
                    fn from_walk<__W: ::guatiao::value::visit::Walker>(
                        __walker: __W,
                    ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                        ::guatiao::__private::read_tree(__walker)
                    }
                }
            }
        }
        Derive::Schema => TokenStream::new(),
    }
}

// --- a struct with named fields: a map ---------------------------------------

/// The write of one field into `__map`, as `set_field` writes it.
fn entry(field: &FieldPlan) -> TokenStream {
    let ident = &field.ident;
    let key = &field.key;
    let access = quote! { &self.#ident };
    let put = |value: TokenStream| {
        quote! {
            ::guatiao::value::visit::MapVisitor::entry(&mut __map, #key, #value)?;
        }
    };
    let write = if let Some(with) = &field.with {
        let put = put(quote! { &__tree });
        quote! {
            let __tree = #with::to_value_in(#access, ::guatiao::Alloc::rust())?;
            #put
        }
    } else if field.as_text {
        if field.optional() {
            let put = put(quote! { &::guatiao::__private::AsText(__value) });
            quote! {
                if let ::core::option::Option::Some(__value) = #access {
                    #put
                }
            }
        } else {
            put(quote! { &::guatiao::__private::AsText(#access) })
        }
    } else if let Some(inner) = &field.inner {
        let put = put(walkable(&quote! { __value }, inner));
        quote! {
            if let ::core::option::Option::Some(__value) = #access {
                #put
            }
        }
    } else {
        put(walkable(&access, &field.ty))
    };
    match &field.skip_if {
        Some(skip_if) => quote! {
            if !#skip_if(#access) {
                #write
            }
        },
        None => write,
    }
}

/// Whether the field is written, as an expression of `bool`.
fn present(field: &FieldPlan) -> Option<TokenStream> {
    let ident = &field.ident;
    let some = field.optional().then(|| quote! { self.#ident.is_some() });
    let kept = field
        .skip_if
        .as_ref()
        .map(|skip_if| quote! { !#skip_if(&self.#ident) });
    match (some, kept) {
        (None, None) => None,
        (Some(a), None) | (None, Some(a)) => Some(a),
        (Some(a), Some(b)) => Some(quote! { (#b && #a) }),
    }
}

fn walk_map(name: &Ident, generics: &syn::Generics, plan: &[FieldPlan]) -> TokenStream {
    let head = walk_head(name, generics);
    let written: Vec<&FieldPlan> = plan.iter().filter(|f| !f.skip).collect();
    let always = written.iter().filter(|f| present(f).is_none()).count();
    let counts = written.iter().filter_map(|f| present(f)).map(|present| {
        quote! { __n += <::core::primitive::usize as ::core::convert::From<::core::primitive::bool>>::from(#present); }
    });
    let entries = written.iter().map(|f| entry(f));
    quote! {
        #head {
            #[allow(clippy::needless_borrow)]
            fn walk<__V: ::guatiao::value::visit::Visitor>(
                &self,
                __visitor: __V,
            ) -> ::core::result::Result<__V::Ok, __V::Error> {
                #[allow(unused_imports)]
                use ::guatiao::__private::{TreeField as _, WalkField as _};
                #[allow(unused_mut)]
                let mut __n: ::core::primitive::usize = #always;
                #(#counts)*
                let mut __map = ::guatiao::value::visit::Visitor::map(
                    __visitor,
                    ::core::option::Option::Some(__n),
                )?;
                #(#entries)*
                ::guatiao::value::visit::MapVisitor::end(__map)
            }
        }
    }
}

/// How one field's value is read, as `read_field` reads it.
fn seed(field: &FieldPlan) -> TokenStream {
    let ty = &field.ty;
    if let Some(with) = &field.with {
        return quote! { ::guatiao::__private::WithSeed(#with::from_value) };
    }
    if field.as_text {
        if field.default.is_none()
            && let Some(inner) = &field.inner
        {
            return quote! { <::guatiao::__private::TextOrNullSeed<#inner>>::new() };
        }
        return quote! { <::guatiao::__private::TextSeed<#ty>>::new() };
    }
    picked(ty)
}

fn read_map(
    name: &Ident,
    generics: &syn::Generics,
    plan: &[FieldPlan],
    deny_unknown: bool,
) -> TokenStream {
    let read_head = head(
        quote!(::guatiao::__private::MapRead),
        quote!(::guatiao::FromWalk),
        name,
        generics,
    );
    let from_head = from_walk_head(name, generics);
    let read: Vec<&FieldPlan> = plan.iter().filter(|f| !f.skip).collect();
    let slots: Vec<Ident> = (0..read.len()).map(|i| format_ident!("__f{i}")).collect();
    let types = read.iter().map(|f| &f.ty);
    let keys: Vec<&String> = read.iter().map(|f| &f.key).collect();
    let indices: Vec<usize> = (0..read.len()).collect();
    let seeds = read.iter().map(|f| seed(f));
    let unknown = if deny_unknown {
        quote! {
            ::core::result::Result::Err(::guatiao::MapError::under(
                ::guatiao::MapError::bad_value("a declared field"),
                __key,
            ))
        }
    } else {
        quote! { ::core::result::Result::Ok(::core::primitive::usize::MAX) }
    };
    let mut slot_of = read.iter().zip(&slots);
    let members = plan.iter().map(|field| {
        let ident = &field.ident;
        if field.skip {
            return quote! { #ident: ::core::default::Default::default(), };
        }
        let (_, slot) = slot_of.next().expect("a slot per field read");
        let key = &field.key;
        let value = if let Some(default) = field.default_expr() {
            quote! {
                match #slot {
                    ::core::option::Option::Some(__value) => __value,
                    ::core::option::Option::None => #default,
                }
            }
        } else if field.optional() {
            quote! {
                match #slot {
                    ::core::option::Option::Some(__value) => __value,
                    ::core::option::Option::None => ::core::option::Option::None,
                }
            }
        } else {
            quote! {
                match #slot {
                    ::core::option::Option::Some(__value) => __value,
                    ::core::option::Option::None => {
                        return ::core::result::Result::Err(::guatiao::MapError::missing(#key));
                    }
                }
            }
        };
        quote! { #ident: #value, }
    });
    quote! {
        #read_head {
            #[allow(clippy::needless_borrow)]
            fn read_map<__M: ::guatiao::value::visit::MapWalker>(
                mut __entries: __M,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                #[allow(unused_imports)]
                use ::guatiao::__private::{TreeSeed as _, WalkSeed as _};
                #( let mut #slots: ::core::option::Option<#types> = ::core::option::Option::None; )*
                while let ::core::option::Option::Some(__index) =
                    ::guatiao::value::visit::MapWalker::next_key(&mut __entries, |__key: &str| {
                        match __key {
                            #( #keys => ::core::result::Result::Ok(#indices), )*
                            _ => #unknown,
                        }
                    })?
                {
                    match __index? {
                        #(
                            #indices => {
                                #slots = ::core::option::Option::Some(
                                    ::guatiao::value::visit::MapWalker::value_seed(
                                        &mut __entries,
                                        #seeds,
                                    )
                                    .map_err(|__e| ::guatiao::MapError::under(__e, #keys))?,
                                );
                            }
                        )*
                        _ => ::guatiao::value::visit::MapWalker::skip_value(&mut __entries)?,
                    }
                }
                ::core::result::Result::Ok(#name {
                    #(#members)*
                })
            }
        }

        #from_head {
            #[allow(clippy::needless_borrow)]
            fn from_walk<__W: ::guatiao::value::visit::Walker>(
                __walker: __W,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                ::guatiao::value::visit::Walker::build(
                    __walker,
                    <::guatiao::__private::ReadMap<Self>>::new(),
                )
            }
        }
    }
}

// --- the shapes that are not a map ------------------------------------------------

fn newtype(
    derive: Derive,
    name: &Ident,
    generics: &syn::Generics,
    member: &Member,
    ty: &Type,
    skipped: &[Ident],
) -> TokenStream {
    if derive == Derive::ToValue {
        let head = walk_head(name, generics);
        let field = walkable(&quote! { &self.#member }, ty);
        return quote! {
            #head {
                #[allow(clippy::needless_borrow)]
                fn walk<__V: ::guatiao::value::visit::Visitor>(
                    &self,
                    __visitor: __V,
                ) -> ::core::result::Result<__V::Ok, __V::Error> {
                    #[allow(unused_imports)]
                    use ::guatiao::__private::{TreeField as _, WalkField as _};
                    ::guatiao::Walk::walk(#field, __visitor)
                }
            }
        };
    }
    let head = from_walk_head(name, generics);
    let seed = picked(ty);
    let built = match member {
        Member::Unnamed(_) => quote! { Self(__inner) },
        Member::Named(field) => quote! {
            Self {
                #field: __inner,
                #( #skipped: ::core::default::Default::default(), )*
            }
        },
    };
    quote! {
        #head {
            #[allow(clippy::needless_borrow)]
            fn from_walk<__W: ::guatiao::value::visit::Walker>(
                __walker: __W,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                #[allow(unused_imports)]
                use ::guatiao::__private::{TreeSeed as _, WalkSeed as _};
                let __inner = ::guatiao::value::visit::Seed::read(#seed, __walker)?;
                ::core::result::Result::Ok(#built)
            }
        }
    }
}

fn tuple(derive: Derive, name: &Ident, generics: &syn::Generics, fields: &[&Field]) -> TokenStream {
    let n = fields.len();
    if derive == Derive::ToValue {
        let head = walk_head(name, generics);
        let items = fields.iter().enumerate().map(|(i, f)| {
            let at = Index::from(i);
            walkable(&quote! { &self.#at }, &f.ty)
        });
        return quote! {
            #head {
                #[allow(clippy::needless_borrow)]
                fn walk<__V: ::guatiao::value::visit::Visitor>(
                    &self,
                    __visitor: __V,
                ) -> ::core::result::Result<__V::Ok, __V::Error> {
                    #[allow(unused_imports)]
                    use ::guatiao::__private::{TreeField as _, WalkField as _};
                    let mut __list = ::guatiao::value::visit::Visitor::list(
                        __visitor,
                        ::core::option::Option::Some(#n),
                    )?;
                    #( ::guatiao::value::visit::ListVisitor::item(&mut __list, #items)?; )*
                    ::guatiao::value::visit::ListVisitor::end(__list)
                }
            }
        };
    }
    let read_head = head(
        quote!(::guatiao::__private::ListRead),
        quote!(::guatiao::FromWalk),
        name,
        generics,
    );
    let from_head = from_walk_head(name, generics);
    let expected = format!("a list of exactly {n} items");
    let items: Vec<Ident> = (0..n).map(|i| format_ident!("__f{i}")).collect();
    let seeds = fields.iter().map(|f| picked(&f.ty));
    let indices = 0..n;
    quote! {
        #read_head {
            #[allow(clippy::needless_borrow)]
            fn read_list<__L: ::guatiao::value::visit::ListWalker>(
                mut __items: __L,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                #[allow(unused_imports)]
                use ::guatiao::__private::{TreeSeed as _, WalkSeed as _};
                let __exactly = || ::guatiao::MapError::bad_value(#expected);
                if ::guatiao::value::visit::ListWalker::len_hint(&__items)
                    .is_some_and(|__len| __len != #n)
                {
                    return ::core::result::Result::Err(__exactly());
                }
                #(
                    let #items = ::guatiao::value::visit::ListWalker::next_seed(&mut __items, #seeds)
                        .map_err(|__e| ::guatiao::MapError::at(__e, #indices))?
                        .ok_or_else(__exactly)?;
                )*
                let __extra: ::core::option::Option<::guatiao::Value> =
                    ::guatiao::value::visit::ListWalker::next(&mut __items)?;
                if __extra.is_some() {
                    return ::core::result::Result::Err(__exactly());
                }
                ::core::result::Result::Ok(Self(#(#items),*))
            }
        }

        #from_head {
            #[allow(clippy::needless_borrow)]
            fn from_walk<__W: ::guatiao::value::visit::Walker>(
                __walker: __W,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                ::guatiao::value::visit::Walker::build(
                    __walker,
                    <::guatiao::__private::ReadList<Self>>::new(),
                )
            }
        }
    }
}

fn unit(derive: Derive, name: &Ident, generics: &syn::Generics) -> TokenStream {
    if derive == Derive::ToValue {
        let head = walk_head(name, generics);
        return quote! {
            #head {
                #[allow(clippy::needless_borrow)]
                fn walk<__V: ::guatiao::value::visit::Visitor>(
                    &self,
                    __visitor: __V,
                ) -> ::core::result::Result<__V::Ok, __V::Error> {
                    ::guatiao::value::visit::Visitor::null(__visitor)
                }
            }
        };
    }
    let head = from_walk_head(name, generics);
    quote! {
        #head {
            #[allow(clippy::needless_borrow)]
            fn from_walk<__W: ::guatiao::value::visit::Walker>(
                __walker: __W,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                <() as ::guatiao::FromWalk>::from_walk(__walker)?;
                ::core::result::Result::Ok(Self)
            }
        }
    }
}
