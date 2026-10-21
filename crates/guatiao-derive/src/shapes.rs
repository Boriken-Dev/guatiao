// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The struct shapes that are not a map: a newtype (and a struct marked
//! `#[map(transparent)]`) is its one field's value, a tuple struct a list
//! by position, a unit struct a null. serde writes each the same way.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Field, Fields, Ident, Index, Member, Type};

use crate::expand::Derive;

/// Refuses `#[map(..)]` and `#[schema(..)]` on a field of a shape that
/// has no keys: a position cannot be renamed, defaulted or described.
pub(crate) fn refuse_field_attributes(derive: Derive, fields: &Fields) -> syn::Result<()> {
    for field in fields {
        if let Some(attr) = field
            .attrs
            .iter()
            .find(|a| a.path().is_ident("map") || a.path().is_ident("schema"))
        {
            return Err(syn::Error::new_spanned(
                attr,
                format!(
                    "{} takes no `#[map(..)]` or `#[schema(..)]` on the field of a tuple, \
                     newtype or transparent struct: a position has no key to rename, \
                     default or describe. Describe the field's own type instead.",
                    derive.spelled()
                ),
            ));
        }
    }
    Ok(())
}

/// How the one field of a transparent struct is reached and built.
pub(crate) struct Transparent<'a> {
    pub(crate) member: Member,
    pub(crate) ty: &'a Type,
    /// The other fields of a named transparent struct: all `#[map(skip)]`,
    /// so each reads as its `Default`.
    pub(crate) skipped: Vec<Ident>,
}

pub(crate) fn emit_transparent(
    derive: Derive,
    head: &TokenStream,
    shape: &Transparent<'_>,
) -> TokenStream {
    let member = &shape.member;
    let ty = shape.ty;
    match derive {
        Derive::ToValue => quote! {
            #head {
                fn to_value_in(
                    &self,
                    __alloc: ::guatiao::Alloc,
                ) -> ::core::result::Result<::guatiao::Value, ::guatiao::ValueError> {
                    <#ty as ::guatiao::ToValue>::to_value_in(&self.#member, __alloc)
                }
            }
        },
        Derive::FromValue => {
            let skipped = &shape.skipped;
            let built = match member {
                Member::Unnamed(_) => quote! {
                    Self(<#ty as ::guatiao::FromValue>::from_value(__value)?)
                },
                Member::Named(name) => quote! {
                    Self {
                        #name: <#ty as ::guatiao::FromValue>::from_value(__value)?,
                        #( #skipped: ::core::default::Default::default(), )*
                    }
                },
            };
            quote! {
                #head {
                    fn from_value(
                        __value: &::guatiao::Value,
                    ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                        ::core::result::Result::Ok(#built)
                    }
                }
            }
        }
        Derive::Schema => quote! {
            #head {
                fn kind(__alloc: ::guatiao::Alloc) -> ::guatiao::schema::KindBuilder {
                    <#ty as ::guatiao::Schema>::kind(__alloc)
                }
            }
        },
    }
}

pub(crate) fn emit_tuple(derive: Derive, head: &TokenStream, fields: &[&Field]) -> TokenStream {
    let n = fields.len();
    let types: Vec<&Type> = fields.iter().map(|f| &f.ty).collect();
    let at: Vec<Index> = (0..n).map(Index::from).collect();
    match derive {
        Derive::ToValue => quote! {
            #head {
                fn to_value_in(
                    &self,
                    __alloc: ::guatiao::Alloc,
                ) -> ::core::result::Result<::guatiao::Value, ::guatiao::ValueError> {
                    let mut __list = ::guatiao::List::new_in(__alloc);
                    #(
                        __list.push_in(
                            <#types as ::guatiao::ToValue>::to_value_in(&self.#at, __alloc)?,
                            __alloc,
                        )?;
                    )*
                    ::core::result::Result::Ok(::guatiao::Value::from(__list))
                }
            }
        },
        Derive::FromValue => {
            let expected = format!("a list of exactly {n} items");
            let reads = (0..n).map(|i| {
                let ty = types[i];
                let item = format_ident!("__f{i}");
                quote! {
                    <#ty as ::guatiao::FromValue>::from_value(#item)
                        .map_err(|__e| ::guatiao::MapError::at(__e, #i))?
                }
            });
            let items: Vec<Ident> = (0..n).map(|i| format_ident!("__f{i}")).collect();
            quote! {
                #head {
                    fn from_value(
                        __value: &::guatiao::Value,
                    ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                        let __list = <&::guatiao::List as ::core::convert::TryFrom<
                            &::guatiao::Value,
                        >>::try_from(__value)?;
                        let [#(#items),*] = &__list[..] else {
                            return ::core::result::Result::Err(
                                ::guatiao::MapError::bad_value(#expected),
                            );
                        };
                        ::core::result::Result::Ok(Self(#(#reads),*))
                    }
                }
            }
        }
        Derive::Schema => quote! {
            #head {
                fn kind(__alloc: ::guatiao::Alloc) -> ::guatiao::schema::KindBuilder {
                    let mut __positions = ::guatiao::__private::alloc::vec::Vec::new();
                    #( __positions.push(<#types as ::guatiao::Schema>::kind(__alloc)); )*
                    ::guatiao::schema::KindBuilder::tuple_in(__alloc, __positions)
                }
            }
        },
    }
}

pub(crate) fn emit_unit(derive: Derive, head: &TokenStream) -> TokenStream {
    match derive {
        Derive::ToValue => quote! {
            #head {
                fn to_value_in(
                    &self,
                    __alloc: ::guatiao::Alloc,
                ) -> ::core::result::Result<::guatiao::Value, ::guatiao::ValueError> {
                    ::core::result::Result::Ok(::guatiao::Value::null())
                }
            }
        },
        Derive::FromValue => quote! {
            #head {
                fn from_value(
                    __value: &::guatiao::Value,
                ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                    <() as ::guatiao::FromValue>::from_value(__value)?;
                    ::core::result::Result::Ok(Self)
                }
            }
        },
        Derive::Schema => quote! {
            #head {
                fn kind(__alloc: ::guatiao::Alloc) -> ::guatiao::schema::KindBuilder {
                    ::guatiao::schema::KindBuilder::null_in(__alloc)
                }
            }
        },
    }
}
