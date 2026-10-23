// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[map(untagged)]`: an enum whose variants are told apart by the kind
//! of value each writes, so no name is stored and any language reads one
//! by looking at the value's kind.
//!
//! | variant | value | schema arm |
//! | --- | --- | --- |
//! | unit | its name, as text; all unit variants together are one arm | a string `enum` |
//! | named fields (at most one) | a map, one key per field | an object |
//! | `Name(T)` | `T`'s value | `T`'s kind |
//!
//! The arms are an `anyOf`. No two may read the same kind: `T`'s kinds are
//! `<T as FromValue>::KINDS`, which only the compiler knows, so the
//! `FromValue` expansion asserts it in a `const` and names both variants
//! when it fails. Reading takes the one arm whose kinds hold the value's.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::{DataEnum, Fields, Ident, Type};

use crate::expand::{
    Derive, FieldPlan, VariantPlan, field_builder, read_field, says_other, set_field,
};
use crate::rename::RenameRule;

/// One arm of the union. The unit variants are one arm, at the place of
/// the first of them.
enum Arm<'a> {
    Names(Vec<&'a VariantPlan>),
    Record(&'a VariantPlan),
    Holds(&'a VariantPlan, &'a Type),
}

impl Arm<'_> {
    /// The kinds the arm reads, as an expression of type `Kinds`.
    fn kinds(&self) -> TokenStream {
        match self {
            Arm::Names(_) => quote! { ::guatiao::Kinds::STRING },
            Arm::Record(_) => quote! { ::guatiao::Kinds::MAP },
            Arm::Holds(_, ty) => quote! { <#ty as ::guatiao::FromValue>::KINDS },
        }
    }

    /// The arm as a compile error names it.
    fn spelled(&self) -> String {
        match self {
            Arm::Names(names) => {
                let names: Vec<String> = names.iter().map(|v| format!("`{}`", v.ident)).collect();
                format!(
                    "the unit variants ({}), which are strings",
                    names.join(", ")
                )
            }
            Arm::Record(v) => format!("`{}`, which is a map", v.ident),
            Arm::Holds(v, _) => format!("`{}`", v.ident),
        }
    }
}

pub(crate) fn expand(
    derive: Derive,
    name: &Ident,
    head: &TokenStream,
    data: &DataEnum,
    rule: Option<RenameRule>,
    generic: bool,
) -> syn::Result<TokenStream> {
    let label = derive.spelled();
    let mut plans = Vec::new();
    for variant in &data.variants {
        if says_other(variant) {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                "`other` has no place in an untagged enum: a string naming no unit \
                 variant is refused rather than kept, since another variant could be \
                 the one a reader in another language would choose.",
            ));
        }
        if let Fields::Unnamed(unnamed) = &variant.fields
            && unnamed.unnamed.len() != 1
        {
            return Err(syn::Error::new_spanned(
                unnamed,
                format!(
                    "{label} needs a variant of an untagged enum to hold one value: \
                     `Name(T)`, or named fields for a map. For a list by position, \
                     hold a tuple: `Name((A, B))`."
                ),
            ));
        }
        let unit = matches!(variant.fields, Fields::Unit);
        let plan = VariantPlan::read(derive, variant, !unit, if unit { rule } else { None })?;
        if !unit && variant.ident.unraw() != plan.stored {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                "`rename` names what a variant is stored as, and an untagged variant \
                 holding a value stores no name. Rename its fields instead.",
            ));
        }
        plans.push(plan);
    }

    let mut records = plans.iter().filter(|p| !p.unit && p.newtype.is_none());
    if let (Some(first), Some(second)) = (records.next(), records.next()) {
        return Err(syn::Error::new(
            second.ident.span(),
            format!(
                "`{}` and `{}` both write a map, and an untagged enum tells its \
                 variants apart by the kind of value each writes, so a reader could \
                 not tell which one a map is. Give the enum a `tag`, or keep one \
                 variant with named fields.",
                first.ident, second.ident
            ),
        ));
    }
    for (i, plan) in plans.iter().enumerate().filter(|(_, p)| p.unit) {
        if let Some(earlier) = plans[..i]
            .iter()
            .find(|e| e.unit && e.stored == plan.stored)
        {
            return Err(syn::Error::new(
                plan.ident.span(),
                format!(
                    "two variants are stored as \"{}\": `{}` and `{}`. Rename one with \
                     `#[map(rename = \"...\")]`.",
                    plan.stored, earlier.ident, plan.ident
                ),
            ));
        }
    }

    let arms = arms(&plans);
    Ok(match derive {
        Derive::ToValue => emit_to(head, &plans),
        Derive::FromValue => emit_from(name, head, &arms, generic),
        Derive::Schema => emit_schema(head, &arms),
    })
}

fn arms(plans: &[VariantPlan]) -> Vec<Arm<'_>> {
    let mut out: Vec<Arm<'_>> = Vec::new();
    for plan in plans {
        if plan.unit {
            if let Some(Arm::Names(names)) = out.iter_mut().find(|a| matches!(a, Arm::Names(_))) {
                names.push(plan);
            } else {
                out.push(Arm::Names(vec![plan]));
            }
        } else if let Some(ty) = &plan.newtype {
            out.push(Arm::Holds(plan, ty));
        } else {
            out.push(Arm::Record(plan));
        }
    }
    out
}

fn emit_to(head: &TokenStream, plans: &[VariantPlan]) -> TokenStream {
    let arms = plans.iter().map(|v| {
        let ident = &v.ident;
        if v.unit {
            let stored = &v.stored;
            return quote! {
                Self::#ident => ::core::result::Result::Ok(::guatiao::Value::from(
                    ::guatiao::Text::new_in(__alloc, #stored)?,
                )),
            };
        }
        if let Some(ty) = &v.newtype {
            return quote! {
                Self::#ident(__f0) => <#ty as ::guatiao::ToValue>::to_value_in(__f0, __alloc),
            };
        }
        // Generated names, never the user's: a field called `__map` would
        // otherwise shadow the expansion's own local.
        let bound: Vec<(&FieldPlan, Ident)> = v
            .fields
            .iter()
            .filter(|f| !f.skip)
            .enumerate()
            .map(|(i, f)| (f, format_ident!("__f{i}")))
            .collect();
        let members = bound.iter().map(|(field, binding)| {
            let member = &field.ident;
            quote! { #member: #binding }
        });
        let sets = bound
            .iter()
            .map(|(field, binding)| set_field(field, &quote! { #binding }));
        quote! {
            Self::#ident { #(#members,)* .. } => {
                let mut __map = ::guatiao::Map::new_in(__alloc);
                #(#sets)*
                ::core::result::Result::Ok(::guatiao::Value::from(__map))
            }
        }
    });
    quote! {
        #head {
            fn to_value_in(
                &self,
                __alloc: ::guatiao::Alloc,
            ) -> ::core::result::Result<
                ::guatiao::Value,
                ::guatiao::ValueError,
            > {
                match self {
                    #(#arms)*
                }
            }
        }
    }
}

/// The assertion that no two arms read one kind. Only a pair involving a
/// `Name(T)` can fail: the other arms' kinds are fixed and differ.
fn checks(name: &Ident, arms: &[Arm<'_>]) -> TokenStream {
    let mut out = TokenStream::new();
    for (i, a) in arms.iter().enumerate() {
        for b in &arms[i + 1..] {
            if !matches!(a, Arm::Holds(..)) && !matches!(b, Arm::Holds(..)) {
                continue;
            }
            let (ka, kb) = (a.kinds(), b.kinds());
            let message = format!(
                "`{name}` is `#[map(untagged)]`, and {} and {} read the same kind of \
                 value, so a reader could not tell which variant a value is. Hold types \
                 whose values differ in kind, or give the enum a `tag`.",
                a.spelled(),
                b.spelled()
            );
            out.extend(quote! {
                if ::guatiao::Kinds::intersects(#ka, #kb) {
                    ::core::panic!(#message);
                }
            });
        }
    }
    out
}

fn emit_from(name: &Ident, head: &TokenStream, arms: &[Arm<'_>], generic: bool) -> TokenStream {
    let checks = checks(name, arms);
    // An item `const` is evaluated by `cargo check` as well as a build;
    // a generic enum's kinds exist only per instance, so it asserts
    // inside the function, when an instance is built.
    let (outside, inside) = if checks.is_empty() {
        (TokenStream::new(), TokenStream::new())
    } else if generic {
        (TokenStream::new(), quote! { const { #checks } })
    } else {
        (quote! { const _: () = { #checks }; }, TokenStream::new())
    };
    let kinds = arms.iter().map(Arm::kinds);
    let tries = arms.iter().map(|arm| {
        let kinds = arm.kinds();
        let read = match arm {
            Arm::Names(names) => {
                let rows = names.iter().map(|v| {
                    let (ident, stored) = (&v.ident, &v.stored);
                    quote! { #stored => ::core::result::Result::Ok(Self::#ident), }
                });
                let stored: Vec<&str> = names.iter().map(|v| v.stored.as_str()).collect();
                let expected = format!("one of {}", stored.join(", "));
                quote! {
                    match <&str as ::core::convert::TryFrom<&::guatiao::Value>>::try_from(
                        __value,
                    )? {
                        #(#rows)*
                        _ => ::core::result::Result::Err(
                            ::guatiao::MapError::bad_value(#expected),
                        ),
                    }
                }
            }
            Arm::Record(v) => {
                let ident = &v.ident;
                let reads = v.fields.iter().map(read_field);
                quote! {
                    {
                        let __map = <&::guatiao::Map as ::core::convert::TryFrom<
                            &::guatiao::Value,
                        >>::try_from(__value)?;
                        ::core::result::Result::Ok(Self::#ident { #(#reads)* })
                    }
                }
            }
            Arm::Holds(v, ty) => {
                let ident = &v.ident;
                quote! {
                    <#ty as ::guatiao::FromValue>::from_value(__value).map(Self::#ident)
                }
            }
        };
        quote! {
            if ::guatiao::Kinds::intersects(#kinds, __found) {
                return #read;
            }
        }
    });
    quote! {
        #head {
            const KINDS: ::guatiao::Kinds = ::guatiao::Kinds::NONE #(.union(#kinds))*;

            fn from_value(
                __value: &::guatiao::Value,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                #inside
                let __found = ::guatiao::__private::kinds_of(__value);
                #(#tries)*
                ::core::result::Result::Err(::guatiao::__private::untagged_refused(
                    <Self as ::guatiao::FromValue>::KINDS,
                ))
            }
        }
        #outside
    }
}

fn emit_schema(head: &TokenStream, arms: &[Arm<'_>]) -> TokenStream {
    let pushes = arms.iter().map(|arm| {
        let (built, plan) = match arm {
            Arm::Names(names) => {
                // No label is written as empty, and the builder leaves an
                // empty one off, as for a unit enum.
                let rows = names.iter().map(|v| {
                    let stored = &v.stored;
                    let label = v.title.as_deref().unwrap_or("");
                    quote! { (#stored, #label), }
                });
                let built = quote! {
                    ::guatiao::schema::KindBuilder::enumeration_in(__alloc, &[#(#rows)*])
                };
                return quote! { __arms.push(#built); };
            }
            Arm::Record(v) => {
                let fields = v.fields.iter().filter(|f| !f.skip).map(|field| {
                    if field.flatten {
                        let ty = &field.ty;
                        return quote! {
                            __fields.extend(::guatiao::__private::fields_of(
                                <#ty as ::guatiao::Schema>::kind(__alloc),
                            ));
                        };
                    }
                    let built = field_builder(field);
                    quote! { __fields.push(#built); }
                });
                let built = quote! {
                    {
                        let mut __fields = ::guatiao::__private::alloc::vec::Vec::new();
                        #(#fields)*
                        ::guatiao::schema::KindBuilder::map_in(__alloc, __fields)
                    }
                };
                (built, *v)
            }
            Arm::Holds(v, ty) => (quote! { <#ty as ::guatiao::Schema>::kind(__alloc) }, *v),
        };
        let mut built = built;
        if let Some(title) = &plan.title {
            built = quote! { ::guatiao::schema::KindBuilder::title(#built, #title) };
        }
        if let Some(description) = &plan.description {
            built = quote! { ::guatiao::schema::KindBuilder::description(#built, #description) };
        }
        quote! { __arms.push(#built); }
    });
    quote! {
        #head {
            fn kind(
                __alloc: ::guatiao::Alloc,
            ) -> ::guatiao::schema::KindBuilder {
                let mut __arms = ::guatiao::__private::alloc::vec::Vec::new();
                #(#pushes)*
                ::guatiao::schema::KindBuilder::union_in(__alloc, __arms)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proc_macro2::TokenStream;
    use quote::quote;

    use crate::expand::{Derive, expand};

    fn from(input: TokenStream) -> String {
        expand(Derive::FromValue, input).to_string()
    }

    #[test]
    fn refusals_are_named() {
        let two = from(quote! { #[map(untagged)] enum E { A { x: u8 }, B { y: u8 } } });
        assert!(two.contains("both write a map"), "{two}");
        let both = from(quote! { #[map(untagged, tag = "t")] enum E { A, B { y: u8 } } });
        assert!(both.contains("not both"), "{both}");
        let other = from(quote! { #[map(untagged)] enum E { A, #[map(other)] O(String) } });
        assert!(other.contains("no place in an untagged enum"), "{other}");
        let pair = from(quote! { #[map(untagged)] enum E { A(u8, u8) } });
        assert!(pair.contains("hold a tuple"), "{pair}");
        let renamed = from(quote! { #[map(untagged)] enum E { #[map(rename = "x")] A(u8) } });
        assert!(renamed.contains("stores no name"), "{renamed}");
        let twice = from(quote! { #[map(untagged)] enum E { A, #[map(rename = "A")] B } });
        assert!(twice.contains("two variants are stored as"), "{twice}");
    }

    #[test]
    fn a_held_type_is_checked_where_its_kinds_are_known() {
        // Fixed arms differ by construction: nothing to assert.
        let fixed = from(quote! { #[map(untagged)] enum E { A, B { x: u8 } } });
        assert!(!fixed.contains("panic"), "{fixed}");
        // A concrete type: an item `const`, which `cargo check` evaluates.
        let concrete = from(quote! { #[map(untagged)] enum E { A, B(u8) } });
        assert!(concrete.contains("const _ : () ="), "{concrete}");
        // A type parameter: inside the reader, per instance.
        let generic = from(quote! { #[map(untagged)] enum E<T> { A(T), B(Vec<T>) } });
        assert!(generic.contains("const {"), "{generic}");
        assert!(!generic.contains("const _ : () ="), "{generic}");
    }
}
