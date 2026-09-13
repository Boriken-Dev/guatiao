// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The expansion, written over [`proc_macro2::TokenStream`] so it can be
//! called from a `#[test]`.
//!
//! # The rule this file is written to
//!
//! **A derive that meets a shape it does not support must say so in one
//! sentence a person can act on.** The failure mode being avoided is the
//! familiar one where an unsupported input produces a page of errors
//! pointing *into the expansion*, at code the reader never wrote. Every
//! rejection here is a [`syn::Error`] carrying a span from the user's own
//! declaration and a message that says what was wrong, why it cannot be
//! supported, and what to do instead.
//!
//! `unsupported_shapes_are_rejected_by_name` at the bottom pins those
//! messages by their text. That is deliberate: a message is only useful
//! if it survives, and nothing else in a build would notice it being
//! replaced by `syn`'s default.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Field, Fields, GenericArgument, Ident, LitStr, PathArguments, Type};

/// Which of the three derives is being expanded.
///
/// They share every check — the shapes one supports, all three support —
/// and differ only in the impl emitted, so sharing the path keeps them
/// from disagreeing about what is legal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Derive {
    ToValue,
    FromValue,
    Schema,
}

impl Derive {
    /// How to spell this derive in a diagnostic. Messages name the derive
    /// the user actually wrote, because "one of the derives on this struct
    /// is unhappy" is not an actionable sentence.
    fn spelled(self) -> &'static str {
        match self {
            Derive::ToValue => "#[derive(ToValue)]",
            Derive::FromValue => "#[derive(FromValue)]",
            Derive::Schema => "#[derive(Schema)]",
        }
    }
}

/// The entry point both macros funnel through.
///
/// Never panics and never returns an empty stream on error: a
/// `compile_error!` carrying the span is what puts the message on the
/// user's declaration instead of on the macro invocation.
pub(crate) fn expand(derive: Derive, input: TokenStream) -> TokenStream {
    match try_expand(derive, input) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error(),
    }
}

fn try_expand(derive: Derive, input: TokenStream) -> syn::Result<TokenStream> {
    let ast: DeriveInput = syn::parse2(input)?;
    let fields = named_fields(derive, &ast)?;

    // Generic parameters are refused rather than guessed at. A generated
    // impl would have to invent the bound for each parameter (`T: ToValue`?
    // `T: ToMap`? both?), and inventing it wrong produces an error inside
    // the expansion -- exactly the class of message this file exists to
    // avoid. Refusing at the declaration costs the user a hand-written
    // impl and tells them so at the point they can act.
    if !ast.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &ast.generics,
            format!(
                "{} does not support generic parameters. The bound to place on each \
                 parameter cannot be inferred here, and guessing it wrong reports an \
                 error inside the expansion rather than on your declaration. Write the \
                 impl by hand.",
                derive.spelled()
            ),
        ));
    }

    let mut plan: Vec<FieldPlan> = Vec::new();
    for field in fields {
        plan.push(FieldPlan::read(derive, field)?);
    }
    reject_duplicate_keys(&plan)?;

    let name = &ast.ident;
    Ok(match derive {
        Derive::ToValue => emit_to(name, &plan),
        Derive::FromValue => emit_from(name, &plan),
        Derive::Schema => emit_schema(name, &plan),
    })
}

/// The named fields of a plain struct, or a message explaining why this
/// particular shape has no map to be.
fn named_fields(derive: Derive, ast: &DeriveInput) -> syn::Result<impl Iterator<Item = &Field>> {
    let label = derive.spelled();
    match &ast.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => Ok(named.named.iter()),
            Fields::Unnamed(unnamed) => Err(syn::Error::new_spanned(
                unnamed,
                format!(
                    "{label} needs named fields: a map is keyed by name, and a tuple \
                     struct has no names to key it by. Give the fields names, or write \
                     the impl by hand and choose the keys yourself."
                ),
            )),
            Fields::Unit => Err(syn::Error::new_spanned(
                &ast.ident,
                format!(
                    "{label} needs named fields, and a unit struct has none. A type \
                     carrying nothing converts to an empty map, which is \
                     `Map::new()` -- no derive required."
                ),
            )),
        },
        Data::Enum(data) => Err(syn::Error::new_spanned(
            data.enum_token,
            format!(
                "{label} supports structs with named fields only. An enum has no one \
                 set of keys -- two variants describe two different maps -- and picking \
                 a tag key to tell them apart (\"type\", \"kind\", a field name) is a \
                 wire-format decision this crate deliberately does not make for you. \
                 Write the impl by hand and choose the tag."
            ),
        )),
        Data::Union(data) => Err(syn::Error::new_spanned(
            data.union_token,
            format!(
                "{label} supports structs with named fields only. A union has no \
                 readable field: which arm is live is not knowable from the type, so \
                 nothing here could decide what to store."
            ),
        )),
    }
}

/// What one field contributes to the expansion.
struct FieldPlan {
    ident: Ident,
    ty: Type,
    /// `Some(T)` when the declared type is a syntactic `Option<T>`. Kept
    /// separately because the three derives need different halves: the
    /// read side names the whole `Option<T>` (whose own `FromValue` impl
    /// handles a stored null), while the write side has already unwrapped
    /// it and must name `T`.
    inner: Option<Type>,
    /// The map key. Meaningless when `skip` is set, and never emitted then.
    key: String,
    skip: bool,
    /// What only the schema reads. Carried on every plan so the three
    /// derives share one parse of one declaration.
    schema: SchemaAttrs,
}

impl FieldPlan {
    fn read(derive: Derive, field: &Field) -> syn::Result<FieldPlan> {
        let ident = field
            .ident
            .clone()
            .expect("named_fields yields only named fields");
        let attrs = FieldAttrs::read(derive, field)?;
        Ok(FieldPlan {
            key: attrs.rename.unwrap_or_else(|| ident.to_string()),
            inner: option_inner(&field.ty).cloned(),
            skip: attrs.skip,
            schema: attrs.schema,
            ty: field.ty.clone(),
            ident,
        })
    }

    fn optional(&self) -> bool {
        self.inner.is_some()
    }
}

#[derive(Default)]
struct FieldAttrs {
    rename: Option<String>,
    skip: bool,
    /// Everything only the schema reads. Collected for every derive, so a
    /// misspelled `#[schema(...)]` is reported by whichever one the user
    /// wrote rather than silently ignored by two of the three.
    schema: SchemaAttrs,
}

/// The presentation and validation hints a field can declare.
///
/// Every one is optional, and a schema with none of them is still correct
/// and still useful -- presentation is optional, substance is not.
#[derive(Default)]
struct SchemaAttrs {
    label: Option<String>,
    /// `#[schema(help = "...")]`, or the field's doc comment when that is
    /// absent. A doc comment is where a person already writes this, and
    /// asking them to write it twice is how the two drift apart.
    help: Option<String>,
    section: Option<String>,
    order: Option<i64>,
    advanced: bool,
    sensitive: bool,
    /// The expression a consumer starts from. Emitted through `ToValue`,
    /// so a default is written in Rust rather than in a value literal.
    default: Option<syn::Expr>,
}

impl FieldAttrs {
    fn read(derive: Derive, field: &Field) -> syn::Result<FieldAttrs> {
        let label = derive.spelled();
        let mut out = FieldAttrs::default();
        let mut rename_span = None;

        out.schema.help = doc_comment(field);

        for attr in &field.attrs {
            if attr.path().is_ident("schema") {
                out.schema.read(label, attr)?;
                continue;
            }
            if !attr.path().is_ident("map") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename") {
                    let literal: LitStr = meta.value()?.parse()?;
                    let text = literal.value();
                    rename_span = Some(literal);
                    out.rename = Some(text);
                    Ok(())
                } else if meta.path.is_ident("skip") {
                    out.skip = true;
                    Ok(())
                } else {
                    Err(meta.error(format!(
                        "unrecognised `#[map(...)]` option. {label} knows two: \
                         `rename = \"...\"` and `skip`."
                    )))
                }
            })?;
        }

        // Contradictory rather than merely redundant, so it is an error
        // and not a warning: whichever one the reader believed, the other
        // silently did not happen.
        if out.skip
            && let Some(literal) = rename_span
        {
            return Err(syn::Error::new_spanned(
                literal,
                "`skip` and `rename` together say nothing: a field that is never \
                 stored has no key to rename. Remove one.",
            ));
        }
        Ok(out)
    }
}

impl SchemaAttrs {
    fn read(&mut self, label: &str, attr: &syn::Attribute) -> syn::Result<()> {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("label") {
                self.label = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("help") {
                self.help = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("section") {
                self.section = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("order") {
                self.order = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);
            } else if meta.path.is_ident("advanced") {
                self.advanced = true;
            } else if meta.path.is_ident("sensitive") {
                self.sensitive = true;
            } else if meta.path.is_ident("default") {
                self.default = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error(format!(
                    "unrecognised `#[schema(...)]` option. {label} knows `label`, \
                     `help`, `section`, `order`, `advanced`, `sensitive` and \
                     `default`; the key and whether the field is required come from \
                     `#[map(...)]` and from the type."
                )));
            }
            Ok(())
        })
    }
}

/// A field's doc comment, as one line of help text.
///
/// Doc comments arrive as `#[doc = "..."]` attributes, one per line, each
/// keeping the leading space the source had. They are joined with spaces
/// rather than newlines: this is help text for a label or a tooltip, and a
/// consumer that wants to wrap it can.
fn doc_comment(field: &Field) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    for attr in &field.attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        let syn::Meta::NameValue(nv) = &attr.meta else {
            continue;
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) = &nv.value
        else {
            continue;
        };
        let line = text.value().trim().to_string();
        // A blank line is a paragraph break. Joined into one line it would
        // show up as a double space, which is a typo rather than a break.
        if !line.is_empty() {
            lines.push(line);
        }
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}

/// Two fields landing on one key is silent data loss — the later `set`
/// replaces the earlier one in place and nothing reports it — so it is
/// refused at compile time.
fn reject_duplicate_keys(plan: &[FieldPlan]) -> syn::Result<()> {
    for (i, field) in plan.iter().enumerate() {
        if field.skip {
            continue;
        }
        if let Some(earlier) = plan[..i]
            .iter()
            .find(|other| !other.skip && other.key == field.key)
        {
            return Err(syn::Error::new(
                field.ident.span(),
                format!(
                    "two fields map to the key \"{}\": `{}` and `{}`. The second would \
                     silently replace the first. Rename one with \
                     `#[map(rename = \"...\")]`.",
                    field.key, earlier.ident, field.ident
                ),
            ));
        }
    }
    Ok(())
}

/// The inner type of a syntactic `Option<T>`.
///
/// Matches on the **last path segment** only, so `Option<T>`,
/// `std::option::Option<T>` and `core::option::Option<T>` are all
/// recognised. The caveat -- an alias is not, and a local type named
/// `Option` wrongly is -- is unavoidable in a proc macro, which has no
/// type information, and is documented on the crate root.
fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    if args.args.len() != 1 {
        return None;
    }
    match args.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

fn emit_to(name: &Ident, plan: &[FieldPlan]) -> TokenStream {
    let sets = plan.iter().filter(|f| !f.skip).map(|field| {
        let ident = &field.ident;
        let key = &field.key;
        // Written as qualified `<Ty as ToValue>::to_value` rather than
        // the inherent-looking `ToValue::to_value`, purely for the
        // DIAGNOSTIC: the qualified form carries the field type's own
        // span, so a field whose type has no conversion is reported at
        // that field. The unqualified form reports it on the
        // `#[derive(..)]` attribute, which tells the reader that
        // something in the struct is wrong and not which thing.
        if let Some(inner) = &field.inner {
            // `None` omits the key entirely rather than storing a null.
            // The contract and its argument live in `guatiao`'s `convert`
            // module; this is the half that implements it.
            //
            // The bound lands on the INNER type because the value in
            // hand has already been unwrapped.
            quote! {
                if let ::core::option::Option::Some(__value) = &self.#ident {
                    ::guatiao::Value::set(
                        &mut __map,
                        #key,
                        <#inner as ::guatiao::ToValue>::to_value(__value, __alloc)?,
                    )?;
                }
            }
        } else {
            let ty = &field.ty;
            quote! {
                ::guatiao::Value::set(
                    &mut __map,
                    #key,
                    <#ty as ::guatiao::ToValue>::to_value(&self.#ident, __alloc)?,
                )?;
            }
        }
    });

    quote! {
        #[automatically_derived]
        impl ::guatiao::ToValue for #name {
            fn to_value(
                &self,
                __alloc: ::guatiao::Alloc,
            ) -> ::core::result::Result<
                ::guatiao::Value,
                ::guatiao::ValueError,
            > {
                let mut __map = ::guatiao::Value::map_in(__alloc);
                #(#sets)*
                ::core::result::Result::Ok(__map)
            }
        }
    }
}

fn emit_from(name: &Ident, plan: &[FieldPlan]) -> TokenStream {
    let reads = plan.iter().map(|field| {
        let ident = &field.ident;
        let ty = &field.ty;
        let key = &field.key;
        if field.skip {
            // Nothing here can invent a value, so `Default` is required
            // -- and requiring it at the use site is what makes the error
            // name the user's own type rather than the expansion.
            return quote! { #ident: ::core::default::Default::default(), };
        }
        // `under` is what turns a leaf error into a dotted path: the
        // value being read does not know its own key, so the reader that
        // knows it adds it here, on the way out.
        if field.optional() {
            quote! {
                #ident: match ::guatiao::convert::find_key(__value, #key) {
                    ::core::option::Option::Some(__field) =>
                        <#ty as ::guatiao::FromValue>::from_value(__field)
                            .map_err(|__e| ::guatiao::MapError::under(__e, #key))?,
                    // Absent reads as `None`, exactly as a stored null
                    // does. See `guatiao`'s `convert` module.
                    ::core::option::Option::None => ::core::option::Option::None,
                },
            }
        } else {
            quote! {
                #ident: <#ty as ::guatiao::FromValue>::from_value(
                    ::guatiao::convert::expect_key(__value, #key)?,
                ).map_err(|__e| ::guatiao::MapError::under(__e, #key))?,
            }
        }
    });

    quote! {
        #[automatically_derived]
        impl ::guatiao::FromValue for #name {
            fn from_value(
                __value: &::guatiao::Value,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                // First, so handing this a string reports THAT rather
                // than reporting every field missing. The error names no
                // key; whoever recursed adds one.
                ::guatiao::convert::expect_map(__value)?;
                ::core::result::Result::Ok(#name {
                    #(#reads)*
                })
            }
        }
    }
}

/// The schema, as builder calls.
///
/// Emitted as calls into `guatiao::schema`'s builders rather than as raw
/// map building, so the vocabulary lives in one place and generated code
/// cannot spell a keyword the reader does not know.
fn emit_schema(name: &Ident, plan: &[FieldPlan]) -> TokenStream {
    let options = plan.iter().filter(|f| !f.skip).map(|field| {
        let key = &field.key;
        let ty = &field.ty;
        let mut built = quote! {
            ::guatiao::schema::OptionBuilder::new_in(__alloc,
                #key,
                <#ty as ::guatiao::Schema>::kind(__alloc),
            )
        };
        let a = &field.schema;
        if let Some(label) = &a.label {
            built = quote! { #built.label(#label) };
        }
        if let Some(help) = &a.help {
            built = quote! { #built.help(#help) };
        }
        if let Some(section) = &a.section {
            built = quote! { #built.section(#section) };
        }
        if let Some(order) = a.order {
            built = quote! { #built.order(#order) };
        }
        if a.advanced {
            built = quote! { #built.advanced() };
        }
        if a.sensitive {
            built = quote! { #built.sensitive() };
        }
        // Not an `Option<T>` means the value has to be there. The
        // declaration already said so; this is only writing it down.
        if !field.optional() {
            built = quote! { #built.required() };
        }
        if let Some(default) = &a.default {
            // Through `ToValue`, so the default is written in Rust and
            // cannot drift from the type it defaults.
            built = quote! {
                #built.default(
                    <#ty as ::guatiao::ToValue>::to_value(&(#default), __alloc),
                )
            };
        }
        quote! { __fields.push(#built); }
    });

    quote! {
        #[automatically_derived]
        impl ::guatiao::Schema for #name {
            fn kind(
                __alloc: ::guatiao::Alloc,
            ) -> ::guatiao::schema::KindBuilder {
                // A plain `Vec` rather than `vec![]`: every path the
                // expansion emits is rooted, and a macro invocation is one
                // more name that has to resolve at the call site.
                let mut __fields = ::std::vec::Vec::new();
                #(#options)*
                ::guatiao::schema::KindBuilder::map_in(__alloc, __fields)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The expansion as text. Every assertion below is on this, which is
    /// what makes the checks independent of the rustc version -- a
    /// `trybuild` snapshot of compiler output is not.
    fn rendered(derive: Derive, input: TokenStream) -> String {
        expand(derive, input).to_string()
    }

    fn to(input: TokenStream) -> String {
        rendered(Derive::ToValue, input)
    }

    fn from(input: TokenStream) -> String {
        rendered(Derive::FromValue, input)
    }

    /// **The failure modes, by their message text.** A derive that
    /// rejects an unsupported shape with a wall of expansion noise is the
    /// thing this asserts against; nothing else in a build would notice
    /// these being replaced by a default.
    #[test]
    fn unsupported_shapes_are_rejected_by_name() {
        let tuple = to(quote! { struct S(u8, u8); });
        assert!(tuple.contains("compile_error"), "{tuple}");
        assert!(tuple.contains("needs named fields"), "{tuple}");
        assert!(tuple.contains("tuple struct"), "{tuple}");
        assert!(tuple.contains("#[derive(ToValue)]"), "{tuple}");

        let unit = to(quote! { struct S; });
        assert!(unit.contains("unit struct has none"), "{unit}");
        assert!(unit.contains("Map::new()"), "{unit}");

        let enumeration = from(quote! { enum E { A, B } });
        assert!(enumeration.contains("compile_error"), "{enumeration}");
        assert!(enumeration.contains("no one set of keys"), "{enumeration}");
        // It must name the derive the user wrote, not the other one.
        assert!(
            enumeration.contains("#[derive(FromValue)]"),
            "{enumeration}"
        );
        assert!(!enumeration.contains("ToValue"), "{enumeration}");

        let union = to(quote! { union U { a: u8, b: u8 } });
        assert!(union.contains("which arm is live"), "{union}");

        let generic = to(quote! { struct S<T> { a: T } });
        assert!(generic.contains("does not support generic"), "{generic}");
        assert!(generic.contains("Write the impl by hand"), "{generic}");

        let lifetime = to(quote! { struct S<'a> { a: &'a str } });
        assert!(
            lifetime.contains("does not support generic"),
            "a lifetime is a generic parameter too: {lifetime}"
        );
    }

    /// Attribute mistakes get their own messages, for the same reason.
    #[test]
    fn attribute_mistakes_are_named_too() {
        let unknown = to(quote! {
            struct S { #[map(renamed = "x")] a: u8 }
        });
        assert!(unknown.contains("unrecognised"), "{unknown}");
        assert!(unknown.contains("rename"), "{unknown}");
        assert!(unknown.contains("skip"), "{unknown}");

        let both = to(quote! {
            struct S { #[map(skip, rename = "x")] a: u8 }
        });
        assert!(both.contains("say nothing"), "{both}");

        // Silent overwrite, refused at compile time.
        let clash = to(quote! {
            struct S { a: u8, #[map(rename = "a")] b: u8 }
        });
        assert!(clash.contains("two fields map to the key"), "{clash}");
        assert!(clash.contains("silently replace"), "{clash}");
    }

    /// A key with a NUL in it is **accepted**.
    ///
    /// A key is pointer and length, so a NUL is an ordinary byte in it.
    /// Refusing one here would be the derive inventing a restriction the
    /// value model does not have — worth pinning, because a NUL in a key
    /// looks like something that ought to be rejected.
    #[test]
    fn a_key_containing_a_nul_is_accepted() {
        let nul = to(quote! {
            struct S { #[map(rename = "a\0b")] a: u8 }
        });
        assert!(!nul.contains("compile_error"), "{nul}");
    }

    /// **Hygiene, asserted rather than assumed.** Generated code that
    /// names a type by a bare identifier compiles in the module that
    /// happens to have imported it and nowhere else, and the person who
    /// hits that has no way to know what is missing.
    #[test]
    fn every_path_the_expansion_emits_is_absolute() {
        // Every field shape at once, so the scan below sees every path
        // the expansion is capable of emitting -- a skipped field is the
        // only thing that reaches `core::default`.
        let declaration = quote! {
            struct S {
                a: u8,
                b: ::core::option::Option<u8>,
                #[map(skip)] c: u8,
            }
        };
        let output = to(declaration.clone()) + &from(declaration);

        // Compared TOKEN by token rather than by substring: `ToMap ::`
        // contains `Map ::`, so a substring search reports a false
        // positive on correct output. `TokenStream::to_string` separates
        // every token with a space, which makes the split exact.
        let tokens: Vec<&str> = output.split_whitespace().collect();
        let rooted = [
            "Alloc",
            "ValueError",
            "FromValue",
            "MapError",
            "ToValue",
            "Value",
            "Map",
            "Option",
            "Result",
            "Default",
            "Some",
            "None",
            "Ok",
            "Err",
        ];
        for (i, token) in tokens.iter().enumerate() {
            if !rooted.contains(token) {
                continue;
            }
            // A name in *type* position after `for`/`impl` is the user's
            // own struct, not ours; every one of ours is reached through
            // a path separator.
            assert_eq!(
                tokens.get(i.wrapping_sub(1)).copied(),
                Some("::"),
                "`{token}` appears unrooted in the expansion, so it would compile only \
                 where the caller happened to import it:\n{output}"
            );
        }

        // WHAT THE EXPANSION IS ALLOWED TO NAME, stated positively.
        //
        // This crate ships standalone: nothing it emits may name an
        // embedding application, and that rule binds GENERATED code
        // specifically -- a macro emitting such an identifier fails the
        // standalone check in every consumer that expands it, while
        // passing here.
        //
        // It is written as an allow-list rather than as a search for the
        // forbidden name because SEARCHING FOR IT WOULD MEAN WRITING IT,
        // in a shipped source file, which is the violation itself. That
        // is not hypothetical: it has been done, and the standalone check
        // then reported the checking crate as the offender.
        //
        // It is also the stronger check. Every path segment the expansion
        // emits must come from this set, so a third crate appearing in
        // generated code is a red test whatever it is called, and the
        // reviewer decides rather than a substring match guessing.
        let mut segments: Vec<&str> = Vec::new();
        for (i, token) in tokens.iter().enumerate() {
            if *token == "::"
                && let Some(next) = tokens.get(i + 1)
                && tokens.get(i + 2) == Some(&"::")
            {
                segments.push(next);
            }
        }
        segments.sort_unstable();
        segments.dedup();

        // A SUBSET rather than an equality, deliberately: an equality
        // would go red on any harmless change to the expansion and train
        // whoever hits it to re-capture the list without reading it,
        // which is how a check stops checking. What must never happen is
        // a name appearing that is NOT here.
        const ALLOWED: &[&str] = &[
            // The two crates generated code reaches into, and the std
            // modules on the way to their items.
            "guatiao",
            "core",
            "default",
            "option",
            "result",
            // The one `guatiao` module generated code names, for the two
            // helpers that have no business at the crate root.
            "convert",
            // Types and traits from `guatiao`.
            "Alloc",
            "Map",
            "ValueError",
            "FromValue",
            "MapError",
            "Value",
            "ToValue",
            "Value",
            // Items from `core`.
            "Default",
            "Option",
            "Result",
        ];
        for segment in &segments {
            assert!(
                ALLOWED.contains(segment),
                "the expansion names `{segment}` in path position, which is outside the \
                 set this derive is allowed to emit. If that is intended, add it to \
                 ALLOWED and say why; if it is a crate name that leaked in, this is the \
                 test that was supposed to catch it:\n{output}"
            );
        }
        // A guard against the guard: if the scan ever found nothing, the
        // loop above would pass while checking nothing at all.
        assert!(
            segments.contains(&"guatiao") && segments.contains(&"core"),
            "the segment scan found no paths, so it proved nothing: {segments:?}"
        );
    }

    /// The happy path emits what each trait needs,
    /// including the second impl that makes nesting work.
    #[test]
    fn a_named_struct_emits_both_impls() {
        let to_side = to(quote! {
            struct S {
                a: u8,
                #[map(rename = "bee")] b: u8,
                #[map(skip)] c: u8,
                d: Option<u8>,
            }
        });
        assert!(
            to_side.contains("impl :: guatiao :: ToValue for S"),
            "{to_side}"
        );
        assert!(
            to_side.contains("\"bee\""),
            "rename must reach the key: {to_side}"
        );
        assert!(
            !to_side.contains("self . c"),
            "a skipped field is never read: {to_side}"
        );
        // The optional field is written conditionally, which is what
        // "None omits the key" means in generated code.
        assert!(
            to_side.contains("if let :: core :: option :: Option :: Some"),
            "{to_side}"
        );
        // Every write takes the allocator, because building a value means
        // allocating one.
        assert!(to_side.contains("__alloc"), "{to_side}");

        let from_side = from(quote! {
            struct S { a: u8, #[map(skip)] c: u8, d: Option<u8> }
        });
        assert!(
            from_side.contains("impl :: guatiao :: FromValue for S"),
            "{from_side}"
        );
        assert!(
            from_side.contains("c : :: core :: default :: Default :: default ()"),
            "a skipped field is defaulted on read: {from_side}"
        );
        assert!(
            from_side.contains("expect_key"),
            "a required field reports a missing key: {from_side}"
        );
        assert!(
            from_side.contains("expect_map"),
            "a value that is not a map is rejected as that, not as every field \
             missing: {from_side}"
        );
    }

    /// A struct with no fields at all is legal and produces an empty map.
    /// Degenerate, but reachable from a macro that generates structs, and
    /// there is nothing wrong with it.
    #[test]
    fn an_empty_named_struct_is_accepted() {
        let output = to(quote! { struct S {} });
        assert!(!output.contains("compile_error"), "{output}");
        assert!(output.contains("Value :: map_in (__alloc)"), "{output}");
    }
}
