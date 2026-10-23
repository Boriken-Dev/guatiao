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
//!
//! # The shapes
//!
//! | declaration | value | schema kind |
//! | --- | --- | --- |
//! | struct with named fields | a map, one key per field | an object |
//! | newtype struct, or `#[map(transparent)]` | the field's value | the field's kind |
//! | tuple struct | a list by position | `prefixItems` |
//! | unit struct | null | `type: null` |
//! | enum of unit variants | the variant's name, as text | a string `enum` |
//! | enum with `#[map(tag = "k")]` | a map: the name under `k`, then the variant's fields, or a `Name(T)` variant's map | a tagged variant |
//! | enum with `#[map(untagged)]` | what the variant writes, told apart by its kind | a union (`untagged.rs`) |
//!
//! The three that are not a map are in `shapes.rs`. A type parameter is
//! bound by the trait derived, as serde bounds one.
//!
//! An enum whose variants carry fields and that names no tag is refused:
//! the key that tells two variants apart is a wire-format decision, and
//! the author makes it rather than this crate.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::{
    Attribute, Data, DataEnum, DeriveInput, Field, Fields, GenericArgument, Ident, LitStr,
    PathArguments, Type, Variant,
};

use crate::rename::RenameRule;

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
    pub(crate) fn spelled(self) -> &'static str {
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
    let label = derive.spelled();
    match &ast.data {
        Data::Struct(data) => {
            // The same reader the enum path uses, so a container-level
            // `#[map(...)]` is refused on a struct rather than ignored --
            // ignoring one would change the wire shape silently.
            let container = ContainerAttrs::read(derive, &ast.attrs, false)?;
            let head = impl_head(derive, &ast.ident, &ast.generics);
            let tokens = expand_struct(derive, &ast.ident, &head, &container, &data.fields)?;
            Ok(crate::krate::reroot(tokens, container.krate.as_ref()))
        }
        Data::Enum(data) => expand_enum(derive, &ast, data),
        Data::Union(data) => Err(syn::Error::new_spanned(
            data.union_token,
            format!(
                "{label} supports structs with named fields and enums. A union has no \
                 readable field: which arm is live is not knowable from the type, so \
                 nothing here could decide what to store."
            ),
        )),
    }
}

/// `impl<..> <trait> for Name<..> where ..`, every type parameter bound
/// by the trait being derived, as serde bounds them: a `T` field of a
/// `Wrapper<T>` converts when `T` does.
fn impl_head(derive: Derive, name: &Ident, generics: &syn::Generics) -> TokenStream {
    let bound = match derive {
        Derive::ToValue => quote!(::guatiao::ToValue),
        Derive::FromValue => quote!(::guatiao::FromValue),
        Derive::Schema => quote!(::guatiao::Schema),
    };
    let mut generics = generics.clone();
    let params: Vec<Ident> = generics.type_params().map(|p| p.ident.clone()).collect();
    let clause = generics.make_where_clause();
    for param in params {
        clause.predicates.push(syn::parse_quote!(#param: #bound));
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        #[automatically_derived]
        impl #impl_generics #bound for #name #ty_generics #where_clause
    }
}

/// A struct of any shape: named fields are a map, unless the struct is
/// `#[map(transparent)]`; one unnamed field is that field's value, several
/// a list; no fields a null.
fn expand_struct(
    derive: Derive,
    name: &Ident,
    head: &TokenStream,
    container: &ContainerAttrs,
    fields: &Fields,
) -> syn::Result<TokenStream> {
    let label = derive.spelled();
    if container.deny_unknown_fields && !matches!(fields, Fields::Named(_)) {
        return Err(syn::Error::new_spanned(
            fields,
            format!(
                "{label} reads `deny_unknown_fields` only on a struct with named fields: nothing else has keys"
            ),
        ));
    }
    match fields {
        Fields::Named(named) if container.transparent => {
            let mut kept = Vec::new();
            let mut skipped = Vec::new();
            for field in &named.named {
                let attrs = FieldAttrs::read(derive, field)?;
                if attrs.skip {
                    skipped.push(field.ident.clone().expect("a named field"));
                } else {
                    kept.push(field);
                }
            }
            let [field] = kept[..] else {
                return Err(syn::Error::new_spanned(
                    named,
                    format!(
                        "{label} needs exactly one field that is not `#[map(skip)]` on a \
                         `#[map(transparent)]` struct: its value is that field's."
                    ),
                ));
            };
            if let Some(attr) = field.attrs.iter().find(|a| a.path().is_ident("schema")) {
                return Err(syn::Error::new_spanned(
                    attr,
                    format!(
                        "{label} takes no `#[schema(..)]` on the field of a transparent struct: describe the field's own type instead."
                    ),
                ));
            }
            let plain = FieldAttrs::read(derive, field)?;
            if plain.rename.is_some()
                || plain.default.is_some()
                || plain.flatten
                || plain.with.is_some()
                || plain.as_text
                || plain.skip_if.is_some()
            {
                return Err(syn::Error::new_spanned(
                    &field.ident,
                    format!(
                        "{label} takes no `#[map(..)]` but `skip` on a transparent struct's fields: the struct has no key of its own."
                    ),
                ));
            }
            let shape = crate::shapes::Transparent {
                member: syn::Member::Named(field.ident.clone().expect("a named field")),
                ty: &field.ty,
                skipped,
            };
            Ok(crate::shapes::emit_transparent(derive, head, &shape))
        }
        Fields::Named(named) => {
            let mut plan: Vec<FieldPlan> = Vec::new();
            for field in &named.named {
                plan.push(FieldPlan::read(derive, field, container.rename_all)?);
            }
            reject_duplicate_keys(&plan)?;
            if container.deny_unknown_fields
                && let Some(flat) = plan.iter().find(|f| f.flatten)
            {
                return Err(syn::Error::new(
                    flat.ident.span(),
                    format!(
                        "{label} cannot read `deny_unknown_fields` beside a `#[map(flatten)]` \
                         field: which keys the flattened member declares is its type's, \
                         and not known here."
                    ),
                ));
            }
            Ok(match derive {
                Derive::ToValue => emit_to(head, &plan),
                Derive::FromValue => emit_from(head, name, &plan, container.deny_unknown_fields),
                Derive::Schema => emit_schema(head, &plan),
            })
        }
        Fields::Unnamed(unnamed) => {
            crate::shapes::refuse_field_attributes(derive, fields)?;
            let all: Vec<&Field> = unnamed.unnamed.iter().collect();
            if let [only] = all[..] {
                let shape = crate::shapes::Transparent {
                    member: syn::Member::Unnamed(syn::Index::from(0)),
                    ty: &only.ty,
                    skipped: Vec::new(),
                };
                return Ok(crate::shapes::emit_transparent(derive, head, &shape));
            }
            if container.transparent {
                return Err(syn::Error::new_spanned(
                    unnamed,
                    format!("{label} needs exactly one field on a `#[map(transparent)]` struct."),
                ));
            }
            Ok(crate::shapes::emit_tuple(derive, head, &all))
        }
        Fields::Unit => {
            if container.transparent {
                return Err(syn::Error::new_spanned(
                    name,
                    format!("{label} needs exactly one field on a `#[map(transparent)]` struct."),
                ));
            }
            Ok(crate::shapes::emit_unit(derive, head))
        }
    }
}

/// What one field contributes to the expansion.
pub(crate) struct FieldPlan {
    pub(crate) ident: Ident,
    pub(crate) ty: Type,
    /// `Some(T)` when the declared type is a syntactic `Option<T>`. Kept
    /// separately because the three derives need different halves: the
    /// read side names the whole `Option<T>` (whose own `FromValue` impl
    /// handles a stored null), while the write side has already unwrapped
    /// it and must name `T`.
    inner: Option<Type>,
    /// The map key. Meaningless when `skip` is set, and never emitted then.
    key: String,
    pub(crate) skip: bool,
    /// What an absent key reads as. With one the field is not required.
    default: Option<MapDefault>,
    /// What only the schema reads. Carried on every plan so the three
    /// derives share one parse of one declaration.
    schema: SchemaAttrs,
    /// `#[map(flatten)]`: the field's own map is written beside the
    /// others, and read from the whole map.
    pub(crate) flatten: bool,
    /// `#[map(with = path)]`: a module with `to_value_in`, `from_value`
    /// and `kind`, converting the field in place of its type's impls.
    with: Option<syn::Path>,
    /// `#[map(skip_serializing_if = path)]`: not written when
    /// `path(&field)` is true.
    skip_if: Option<syn::Path>,
    /// `#[map(as_text)]`: written with `Display`, read with `FromStr`,
    /// described as a string.
    as_text: bool,
}

/// `#[map(default)]` or `#[map(default = <expr>)]`.
enum MapDefault {
    /// The field type's own `Default`.
    Trait,
    /// An expression of the field's type.
    Expr(syn::Expr),
}

impl FieldPlan {
    /// `rule` is the struct's `rename_all`; a field's own `rename` wins.
    fn read(derive: Derive, field: &Field, rule: Option<RenameRule>) -> syn::Result<FieldPlan> {
        let ident = field
            .ident
            .clone()
            .expect("only named fields reach a FieldPlan");
        let attrs = FieldAttrs::read(derive, field)?;
        Ok(FieldPlan {
            key: attrs.rename.unwrap_or_else(|| {
                let name = ident.unraw().to_string();
                rule.map_or(name.clone(), |rule| rule.field(&name))
            }),
            inner: option_inner(&field.ty).cloned(),
            skip: attrs.skip,
            default: attrs.default,
            schema: attrs.schema,
            flatten: attrs.flatten,
            with: attrs.with,
            skip_if: attrs.skip_if,
            as_text: attrs.as_text,
            ty: field.ty.clone(),
            ident,
        })
    }

    fn optional(&self) -> bool {
        self.inner.is_some()
    }

    /// The value an absent key reads as, as an expression of the field's
    /// type. Qualified and typed so a type with no `Default`, or an
    /// expression of another type, is reported at the field.
    fn default_expr(&self) -> Option<TokenStream> {
        let ty = &self.ty;
        match self.default.as_ref()? {
            MapDefault::Trait => Some(quote! { <#ty as ::core::default::Default>::default() }),
            MapDefault::Expr(expr) => Some(quote! {
                {
                    let __default: #ty = #expr;
                    __default
                }
            }),
        }
    }
}

#[derive(Default)]
struct FieldAttrs {
    rename: Option<String>,
    skip: bool,
    default: Option<MapDefault>,
    flatten: bool,
    with: Option<syn::Path>,
    skip_if: Option<syn::Path>,
    as_text: bool,
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
    /// `#[schema(title = "...")]`: JSON Schema's own keyword, named after
    /// it.
    title: Option<String>,
    /// `#[schema(description = "...")]`, or the field's doc comment when
    /// that is absent. A doc comment is where a person already writes
    /// this, and asking them to write it twice is how the two drift apart.
    description: Option<String>,
    section: Option<String>,
    order: Option<i64>,
    advanced: bool,
    sensitive: bool,
    /// JSON Schema's shape keywords, chained onto the KIND: they say what
    /// the type's values look like, so they go where the kind is built.
    min_length: Option<u64>,
    max_length: Option<u64>,
    pattern: Option<String>,
    format: Option<String>,
    multiple_of: Option<f64>,
    min_items: Option<u64>,
    max_items: Option<u64>,
    /// JSON Schema's meta-data annotations, on the FIELD.
    read_only: bool,
    deprecated: bool,
    /// Each an expression of any type with a `ToValue`, so an example is
    /// written in Rust like a default is.
    examples: Vec<syn::Expr>,
    /// The expression a consumer starts from. Emitted through `ToValue`,
    /// so a default is written in Rust rather than in a value literal.
    default: Option<syn::Expr>,
}

impl FieldAttrs {
    fn read(derive: Derive, field: &Field) -> syn::Result<FieldAttrs> {
        let label = derive.spelled();
        let mut out = FieldAttrs::default();
        let mut rename_span = None;
        let mut default_span = None;

        out.schema.description = doc_comment(&field.attrs);

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
                } else if meta.path.is_ident("flatten") {
                    out.flatten = true;
                    Ok(())
                } else if meta.path.is_ident("with") {
                    out.with = Some(path_value(&meta)?);
                    Ok(())
                } else if meta.path.is_ident("skip_serializing_if") {
                    out.skip_if = Some(path_value(&meta)?);
                    Ok(())
                } else if meta.path.is_ident("as_text") {
                    out.as_text = true;
                    Ok(())
                } else if meta.path.is_ident("default") {
                    if out.default.is_some() {
                        return Err(meta.error("`default` is given twice. Keep one."));
                    }
                    default_span = Some(meta.path.clone());
                    out.default = Some(if meta.input.peek(syn::Token![=]) {
                        MapDefault::Expr(meta.value()?.parse()?)
                    } else {
                        MapDefault::Trait
                    });
                    Ok(())
                } else {
                    Err(meta.error(format!(
                        "unrecognised `#[map(...)]` option. {label} knows `rename = \"...\"`, \
                         `skip`, `default` or `default = <expr>`, `flatten`, `with = path`, \
                         `as_text` and `skip_serializing_if = path`. Not `alias`: a schema refuses \
                         every key it does not declare, so an alias would read a value \
                         its own schema refuses."
                    )))
                }
            })?;
        }

        if out.flatten
            && (out.skip
                || rename_span.is_some()
                || default_span.is_some()
                || out.with.is_some()
                || out.as_text
                || out.skip_if.is_some())
        {
            return Err(syn::Error::new_spanned(
                &field.ident,
                "`flatten` stands alone: a flattened field has no key of its own to \
                 rename, skip, default or convert, since its type's keys are written \
                 beside this struct's.",
            ));
        }
        if out.flatten && option_inner(&field.ty).is_some() {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "`flatten` takes a type that writes a map, and an `Option` writes a null \
                 when it is `None`, which has no keys to write beside the others.",
            ));
        }
        if out.as_text && out.with.is_some() {
            return Err(syn::Error::new_spanned(
                &field.ident,
                "`as_text` and `with` both say how the field converts. Keep one: \
                 `as_text` for a type with `Display` and `FromStr`, `with` for any \
                 other shape.",
            ));
        }
        if out.with.is_some() && option_inner(&field.ty).is_some() {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "`with` converts a whole field, and an `Option` field omits its key, \
                 which the module cannot see. Convert an `Option<T>` in the module, on \
                 a field of that module's own type.",
            ));
        }
        if out.skip_if.is_some() && default_span.is_none() && option_inner(&field.ty).is_none() {
            return Err(syn::Error::new_spanned(
                &field.ident,
                "`skip_serializing_if` leaves the key out, so reading it back needs \
                 `#[map(default)]` or `#[map(default = <expr>)]` beside it: an absent \
                 key must read as something.",
            ));
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
        if let Some(span) = default_span {
            if out.skip {
                return Err(syn::Error::new_spanned(
                    span,
                    "`skip` and `default` together say nothing: a field that is never \
                     stored always reads as its `Default`. Remove one.",
                ));
            }
            if option_inner(&field.ty).is_some() {
                return Err(syn::Error::new_spanned(
                    span,
                    "an `Option` field is already optional: an absent key reads as \
                     `None`, and `None` is written as an absent key, so a default \
                     could never be written back. Make the field a plain `T` to \
                     give it a default.",
                ));
            }
            if out.schema.default.is_some() {
                return Err(syn::Error::new_spanned(
                    span,
                    "`#[map(default)]` and `#[schema(default = ..)]` both name this \
                     field's default. `#[map(default = <expr>)]` says it once: an \
                     absent key reads as it, and the schema carries it.",
                ));
            }
        }
        Ok(out)
    }
}

impl SchemaAttrs {
    fn read(&mut self, label: &str, attr: &syn::Attribute) -> syn::Result<()> {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("title") {
                self.title = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("description") {
                self.description = Some(meta.value()?.parse::<LitStr>()?.value());
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
            } else if meta.path.is_ident("min_length") {
                self.min_length = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);
            } else if meta.path.is_ident("max_length") {
                self.max_length = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);
            } else if meta.path.is_ident("pattern") {
                self.pattern = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("format") {
                self.format = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("multiple_of") {
                // Either spelling of a number: `5` and `0.25` both mean a
                // step, and making a person write `5.0` for an integer
                // field would be ceremony.
                let step = match meta.value()?.parse::<syn::Lit>()? {
                    syn::Lit::Int(i) => i.base10_parse::<f64>()?,
                    syn::Lit::Float(f) => f.base10_parse::<f64>()?,
                    other => {
                        return Err(syn::Error::new_spanned(other, "a number: `5` or `0.25`"));
                    }
                };
                if !(step > 0.0 && step.is_finite()) {
                    return Err(meta.error("`multiple_of` is a step greater than zero"));
                }
                self.multiple_of = Some(step);
            } else if meta.path.is_ident("min_items") {
                self.min_items = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);
            } else if meta.path.is_ident("max_items") {
                self.max_items = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);
            } else if meta.path.is_ident("read_only") {
                self.read_only = true;
            } else if meta.path.is_ident("deprecated") {
                self.deprecated = true;
            } else if meta.path.is_ident("examples") {
                let content;
                syn::parenthesized!(content in meta.input);
                self.examples =
                    syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated(
                        &content,
                    )?
                    .into_iter()
                    .collect();
            } else {
                return Err(meta.error(format!(
                    "unrecognised `#[schema(...)]` option. {label} knows `title`, \
                     `description`, `section`, `order`, `advanced`, `sensitive`, \
                     `default`, `read_only`, `deprecated` and `examples(..)` about the \
                     field, and `min_length`, `max_length`, `pattern`, `format`, \
                     `multiple_of`, `min_items` and `max_items` about its values; the \
                     key and whether the field is required come from `#[map(...)]` and \
                     from the type."
                )));
            }
            Ok(())
        })
    }
}

/// The path a `#[map(name = ..)]` names: a path, or a string holding one,
/// as serde spells it.
fn path_value(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<syn::Path> {
    let value = meta.value()?;
    if value.peek(LitStr) {
        value.parse::<LitStr>()?.parse()
    } else {
        value.parse()
    }
}

/// A doc comment, as one line of text.
///
/// Doc comments arrive as `#[doc = "..."]` attributes, one per line, each
/// keeping the leading space the source had. They are joined with spaces
/// rather than newlines: this is text for a label or a tooltip, and a
/// consumer that wants to wrap it can.
fn doc_comment(attrs: &[Attribute]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    for attr in attrs {
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
        if field.skip || field.flatten {
            continue;
        }
        if let Some(earlier) = plan[..i]
            .iter()
            .find(|other| !other.skip && !other.flatten && other.key == field.key)
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

// --- one field, three ways ---------------------------------------------
//
// A struct's fields and a tagged variant's fields are the same thing, so
// they are emitted by the same three functions. What differs is only how
// the field is reached: `&self.name` in a struct, a pattern binding in a
// variant.

/// Writes one field into `__map`. `access` borrows the field.
pub(crate) fn set_field(field: &FieldPlan, access: &TokenStream) -> TokenStream {
    let ty = &field.ty;
    if field.flatten {
        return quote! {
            ::guatiao::__private::flatten_into(
                &mut __map,
                <#ty as ::guatiao::ToValue>::to_value_in(#access, __alloc)?,
                __alloc,
            )?;
        };
    }
    let set = set_value(field, access);
    match &field.skip_if {
        Some(skip_if) => quote! {
            if !#skip_if(#access) {
                #set
            }
        },
        None => set,
    }
}

/// The write of one keyed field, `None` omitted.
fn set_value(field: &FieldPlan, access: &TokenStream) -> TokenStream {
    let key = &field.key;
    if let Some(with) = &field.with {
        return quote! {
            ::guatiao::Map::set_in(&mut __map, #key, #with::to_value_in(#access, __alloc)?, __alloc)?;
        };
    }
    if field.as_text {
        // Its `Display` text; `None` omits the key, as for any `Option`.
        let set = |value: TokenStream| {
            quote! {
                ::guatiao::Map::set_in(
                    &mut __map,
                    #key,
                    ::guatiao::__private::text_of(#value, __alloc)?,
                    __alloc,
                )?;
            }
        };
        return if field.optional() {
            let set = set(quote! { __value });
            quote! {
                if let ::core::option::Option::Some(__value) = #access {
                    #set
                }
            }
        } else {
            set(access.clone())
        };
    }
    // Written as qualified `<Ty as ToValue>::to_value` rather than the
    // inherent-looking `ToValue::to_value`, purely for the DIAGNOSTIC: the
    // qualified form carries the field type's own span, so a field whose
    // type has no conversion is reported at that field. The unqualified
    // form reports it on the `#[derive(..)]` attribute, which tells the
    // reader that something in the struct is wrong and not which thing.
    if let Some(inner) = &field.inner {
        // `None` omits the key entirely rather than storing a null. The
        // contract and its argument live in `guatiao`'s `convert` module;
        // this is the half that implements it.
        //
        // The bound lands on the INNER type because the value in hand has
        // already been unwrapped.
        quote! {
            if let ::core::option::Option::Some(__value) = #access {
                ::guatiao::Map::set_in(
                    &mut __map,
                    #key,
                    <#inner as ::guatiao::ToValue>::to_value_in(__value, __alloc)?,
                    __alloc,
                )?;
            }
        }
    } else {
        let ty = &field.ty;
        quote! {
            ::guatiao::Map::set_in(
                &mut __map,
                #key,
                <#ty as ::guatiao::ToValue>::to_value_in(#access, __alloc)?,
                __alloc,
            )?;
        }
    }
}

/// Reads one field out of the map `__value`, as a struct-literal member.
pub(crate) fn read_field(field: &FieldPlan) -> TokenStream {
    let ident = &field.ident;
    let ty = &field.ty;
    let key = &field.key;
    if field.skip {
        // Nothing here can invent a value, so `Default` is required -- and
        // requiring it at the use site is what makes the error name the
        // user's own type rather than the expansion.
        return quote! { #ident: ::core::default::Default::default(), };
    }
    if field.flatten {
        // The whole map: a flattened member's keys are this one's.
        return quote! { #ident: <#ty as ::guatiao::FromValue>::from_value(__value)?, };
    }
    let read = match &field.with {
        Some(with) => quote! { #with::from_value },
        None if field.as_text => quote! { ::guatiao::__private::from_text },
        None => quote! { <#ty as ::guatiao::FromValue>::from_value },
    };
    // `under` is what turns a leaf error into a dotted path: the value
    // being read does not know its own key, so the reader that knows it
    // adds it here, on the way out.
    if let Some(default) = field.default_expr() {
        return quote! {
            #ident: match __map.get(#key) {
                ::core::option::Option::Some(__field) =>
                    #read(__field)
                        .map_err(|__e| ::guatiao::MapError::under(__e, #key))?,
                ::core::option::Option::None => #default,
            },
        };
    }
    if field.optional() {
        let read = if field.as_text {
            quote! { ::guatiao::__private::from_text_or_null }
        } else {
            quote! { <#ty as ::guatiao::FromValue>::from_value }
        };
        quote! {
            #ident: match __map.get(#key) {
                ::core::option::Option::Some(__field) =>
                    #read(__field)
                        .map_err(|__e| ::guatiao::MapError::under(__e, #key))?,
                // Absent reads as `None`, exactly as a stored null does.
                // See `guatiao`'s `convert` module.
                ::core::option::Option::None => ::core::option::Option::None,
            },
        }
    } else {
        quote! {
            #ident: #read(
                ::guatiao::Map::required(__map, #key)?,
            ).map_err(|__e| ::guatiao::MapError::under(__e, #key))?,
        }
    }
}

/// One field as a `FieldBuilder` expression.
///
/// Emitted as calls into `guatiao::schema`'s builders rather than as raw
/// map building, so the vocabulary lives in one place and generated code
/// cannot spell a keyword the reader does not know.
pub(crate) fn field_builder(field: &FieldPlan) -> TokenStream {
    let key = &field.key;
    let ty = &field.ty;
    let a = &field.schema;
    // The shape keywords chain onto the KIND, before it becomes a field.
    // Inherent methods, called by path for the same reason as below.
    let mut kind = match &field.with {
        Some(with) => quote! { #with::kind(__alloc) },
        None if field.as_text => {
            let nullable = field.optional();
            quote! { ::guatiao::__private::text_kind(__alloc, #nullable) }
        }
        None => quote! { <#ty as ::guatiao::Schema>::kind(__alloc) },
    };
    for (method, n) in [
        ("min_length", a.min_length),
        ("max_length", a.max_length),
        ("min_items", a.min_items),
        ("max_items", a.max_items),
    ] {
        if let Some(n) = n {
            let method = syn::Ident::new(method, proc_macro2::Span::call_site());
            kind = quote! { ::guatiao::schema::KindBuilder::#method(#kind, #n) };
        }
    }
    if let Some(pattern) = &a.pattern {
        kind = quote! { ::guatiao::schema::KindBuilder::pattern(#kind, #pattern) };
    }
    if let Some(format) = &a.format {
        kind = quote! { ::guatiao::schema::KindBuilder::format(#kind, #format) };
    }
    if let Some(step) = a.multiple_of {
        kind = quote! { ::guatiao::schema::KindBuilder::multiple_of(#kind, #step) };
    }
    let mut built = quote! {
        ::guatiao::schema::FieldBuilder::new_in(__alloc, #key, #kind)
    };
    // Qualified, not `.title(..)`. Some of these are trait methods, and
    // method syntax would need the trait in scope at the EXPANSION site --
    // somebody else's crate, which generated code may not assume anything
    // about. The same reason every path here is rooted at `::guatiao`.
    if let Some(title) = &a.title {
        built = quote! {
            ::guatiao::schema::FieldBuilder::title(#built, #title)
        };
    }
    if let Some(description) = &a.description {
        built = quote! {
            ::guatiao::schema::FieldBuilder::description(#built, #description)
        };
    }
    // The presentation keys go through `Extras`, the general door, and are
    // spelled out rather than named: they are `guatiao-intake`'s vocabulary
    // (`guatiao_intake::vocab::X_SECTION` and its two siblings), and neither
    // `guatiao` nor a crate deriving a schema has to depend on that crate
    // to carry one. THIS is the seam -- a rename there is a rename here,
    // and `crates/guatiao-intake/tests/form_derive.rs` is what notices.
    //
    // The allocator comes back out of the builder, so a hint lands in the
    // same arena the schema is being built into.
    if let Some(section) = &a.section {
        built = quote! {
            ::guatiao::schema::Extras::extra(
                #built,
                "x-section",
                ::guatiao::Text::new_in(__alloc, #section).map(::guatiao::Value::from),
            )
        };
    }
    if let Some(order) = a.order {
        built = quote! {
            ::guatiao::schema::Extras::extra(
                #built,
                "x-order",
                ::guatiao::Number::new_in(__alloc, &#order.to_string())
                    .map(::guatiao::Value::from),
            )
        };
    }
    if a.advanced {
        built = quote! {
            ::guatiao::schema::Extras::extra(
                #built,
                "x-advanced",
                ::core::result::Result::Ok(::guatiao::Value::from(true)),
            )
        };
    }
    if a.sensitive {
        built = quote! {
            ::guatiao::schema::FieldBuilder::sensitive(#built)
        };
    }
    if a.read_only {
        built = quote! { ::guatiao::schema::FieldBuilder::read_only(#built) };
    }
    if a.deprecated {
        built = quote! { ::guatiao::schema::FieldBuilder::deprecated(#built) };
    }
    if !a.examples.is_empty() {
        // Built through `ToValue` into the schema's own arena, with the
        // first failure kept by the builder -- the same route a default
        // takes, and the reason this is not `FieldBuilder::examples`,
        // which takes values already built somewhere else.
        let examples = &a.examples;
        built = quote! {
            ::guatiao::schema::Extras::extra(
                #built,
                ::guatiao::schema::vocab::EXAMPLES,
                {
                    use ::guatiao::ToValue as _;
                    let mut __examples = ::guatiao::List::new_in(__alloc);
                    let mut __ok: ::core::result::Result<(), ::guatiao::ValueError> =
                        ::core::result::Result::Ok(());
                    #(
                        if __ok.is_ok() {
                            __ok = (#examples)
                                .to_value_in(__alloc)
                                .and_then(|__v| __examples.push_in(__v, __alloc));
                        }
                    )*
                    __ok.map(|()| ::guatiao::Value::from(__examples))
                },
            )
        };
    }
    // Neither an `Option<T>` nor defaulted means the value has to be
    // there. The declaration already said so; this is only writing it
    // down.
    if !field.optional() && field.default.is_none() {
        built = quote! { #built.required() };
    }
    // A default is written the way the field is: as its text when the
    // field crosses as text, which is all such a type offers.
    let write = if field.as_text {
        quote! { ::guatiao::__private::text_of }
    } else {
        quote! { <#ty as ::guatiao::ToValue>::to_value_in }
    };
    if let Some(default) = field.default_expr() {
        // The value an absent key reads as is the one the schema declares,
        // so a consumer filling a form starts from what the reader uses.
        built = quote! {
            #built.default_checked(
                #write(&#default, __alloc),
            )
        };
    }
    if let Some(default) = &a.default {
        // Through `ToValue`, so the default is written in Rust and cannot
        // drift from the type it defaults. `default_checked`, because this
        // has a `Result` and nowhere to put a failure: the builder keeps it
        // and `finish` reports it.
        built = quote! {
            #built.default_checked(
                #write(&(#default), __alloc),
            )
        };
    }
    built
}

// --- structs -----------------------------------------------------------

fn emit_to(head: &TokenStream, plan: &[FieldPlan]) -> TokenStream {
    let sets = plan.iter().filter(|f| !f.skip).map(|field| {
        let ident = &field.ident;
        set_field(field, &quote! { &self.#ident })
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
                let mut __map = ::guatiao::Map::new_in(__alloc);
                #(#sets)*
                ::core::result::Result::Ok(::guatiao::Value::from(__map))
            }
        }
    }
}

fn emit_from(
    head: &TokenStream,
    name: &Ident,
    plan: &[FieldPlan],
    deny_unknown: bool,
) -> TokenStream {
    let reads = plan.iter().map(read_field);
    // `deny_unknown_fields`: every key is one a field declares.
    let strict = deny_unknown.then(|| {
        let keys = plan.iter().filter(|f| !f.skip).map(|f| &f.key);
        quote! {
            for __entry in __map {
                if ![#(#keys),*].contains(&__entry.key()) {
                    return ::core::result::Result::Err(::guatiao::MapError::under(
                        ::guatiao::MapError::bad_value("a declared field"),
                        __entry.key(),
                    ));
                }
            }
        }
    });

    quote! {
        #head {
            const KINDS: ::guatiao::Kinds = ::guatiao::Kinds::MAP;

            fn from_value(
                __value: &::guatiao::Value,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                // First, so handing this a string reports THAT rather
                // than reporting every field missing. The error names no
                // key; whoever recursed adds one.
                let __map = <&::guatiao::Map as ::core::convert::TryFrom<
                    &::guatiao::Value,
                >>::try_from(__value)?;
                #strict
                ::core::result::Result::Ok(#name {
                    #(#reads)*
                })
            }
        }
    }
}

/// The schema, as builder calls.
fn emit_schema(head: &TokenStream, plan: &[FieldPlan]) -> TokenStream {
    let fields = plan.iter().filter(|f| !f.skip).map(|field| {
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

    quote! {
        #head {
            fn kind(
                __alloc: ::guatiao::Alloc,
            ) -> ::guatiao::schema::KindBuilder {
                // A plain `Vec` rather than `vec![]`: every path the
                // expansion emits is rooted, and a macro invocation is one
                // more name that has to resolve at the call site.
                let mut __fields = ::guatiao::__private::alloc::vec::Vec::new();
                #(#fields)*
                ::guatiao::schema::KindBuilder::map_in(__alloc, __fields)
            }
        }
    }
}

// --- enums -------------------------------------------------------------

/// What `#[map(...)]` on the type itself says.
///
/// One reader for a struct and an enum, because the two must disagree
/// about nothing: a key the container does not know is refused either
/// way, and `tag` -- which only an enum can mean -- is refused by name on
/// a struct rather than accepted and ignored.
#[derive(Default)]
struct ContainerAttrs {
    /// `#[map(tag = "...")]`: the key a variant's name is stored under.
    /// The one wire-format decision an enum can need, so the author makes
    /// it rather than this crate.
    tag: Option<String>,
    /// `#[map(crate = <path>)]`: where the expansion finds `guatiao`,
    /// for a consumer that reaches it through a re-export.
    krate: Option<syn::Path>,
    /// `#[map(rename_all = "..")]`: the case rule for every field of a
    /// struct, or every variant of an enum.
    rename_all: Option<RenameRule>,
    /// `#[map(transparent)]`, on a struct: its value is its one field's.
    transparent: bool,
    /// `#[map(deny_unknown_fields)]`, on a struct: reading refuses a key
    /// no field declares, as the schema does.
    deny_unknown_fields: bool,
    /// `#[map(untagged)]`, on an enum: its variants are told apart by
    /// the kind of value each writes, and no name is stored.
    untagged: bool,
}

impl ContainerAttrs {
    fn read(derive: Derive, attrs: &[Attribute], is_enum: bool) -> syn::Result<ContainerAttrs> {
        let label = derive.spelled();
        let mut out = ContainerAttrs::default();
        for attr in attrs {
            if !attr.path().is_ident("map") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("crate") {
                    out.krate = Some(meta.value()?.parse()?);
                    Ok(())
                } else if meta.path.is_ident("rename_all") {
                    out.rename_all = Some(RenameRule::parse(&meta.value()?.parse::<LitStr>()?)?);
                    Ok(())
                } else if meta.path.is_ident("transparent") && !is_enum {
                    out.transparent = true;
                    Ok(())
                } else if meta.path.is_ident("deny_unknown_fields") && !is_enum {
                    out.deny_unknown_fields = true;
                    Ok(())
                } else if meta.path.is_ident("untagged") && is_enum {
                    out.untagged = true;
                    Ok(())
                } else if meta.path.is_ident("tag") && is_enum {
                    out.tag = Some(meta.value()?.parse::<LitStr>()?.value());
                    Ok(())
                } else if meta.path.is_ident("tag") {
                    Err(meta.error(format!(
                        "`tag` names the key a VARIANT's name is stored under, so \
                         {label} has nothing to do with one on a struct: a struct is \
                         one shape and has no variants to tell apart."
                    )))
                } else if is_enum {
                    // Refused rather than ignored: a misspelled `tag`
                    // silently ignored would change the wire shape.
                    Err(meta.error(format!(
                        "unrecognised `#[map(...)]` option on an enum. {label} knows \
                         four: `tag = \"...\"`, the key a variant's name is stored under, \
                         `untagged`, for variants told apart by the kind of value they \
                         write, `rename_all = \"...\"`, the case of every variant's name, \
                         and `crate = <path>`, where `guatiao` is re-exported."
                    )))
                } else {
                    Err(meta.error(format!(
                        "unrecognised `#[map(...)]` option on a struct. {label} takes \
                         `rename_all = \"...\"`, `transparent`, `deny_unknown_fields` and \
                         `crate = <path>` here: `rename = \"...\"`, `skip` and `default` \
                         go on a field, and `tag` is an enum's."
                    )))
                }
            })?;
        }
        Ok(out)
    }
}

/// One variant, read once and shared by all three derives.
pub(crate) struct VariantPlan {
    pub(crate) ident: Ident,
    /// What is stored for it: the variant's own name, or its
    /// `#[map(rename = "...")]`.
    pub(crate) stored: String,
    /// A unit variant, which is constructed and matched without braces.
    pub(crate) unit: bool,
    /// Named fields, in declaration order. Empty for a unit variant.
    pub(crate) fields: Vec<FieldPlan>,
    /// The text shown for it: an arm's `title`, or a choice's entry in
    /// `x-enum-labels` -- which is why a choice declares it as `label`.
    pub(crate) title: Option<String>,
    /// An arm's `description`. A choice has none.
    pub(crate) description: Option<String>,
    /// `#[map(other)]`: the catch-all of a unit enum, holding a name no
    /// other variant is stored as. The type of its one field.
    other: Option<Type>,
    /// `Name(T)`: a tagged enum writes `T`'s map beside the tag, an
    /// untagged one `T`'s value.
    pub(crate) newtype: Option<Type>,
}

impl VariantPlan {
    /// `tagged` decides both what a doc comment is and what the attribute
    /// is called, because the two shapes write different keys. An arm is a
    /// subschema, so it takes `title` and `description`, JSON Schema's own
    /// keywords; a choice is one entry in `x-enum-labels`, so it takes
    /// `label`, that key's own word.
    ///
    /// **A doc comment fills the most descriptive human slot the thing
    /// has**: an arm has a description as well as a title, so its doc
    /// comment is the description, the same as a struct field's; a choice
    /// has only a label, so its doc comment is the label.
    ///
    /// `rule` is the enum's `rename_all`, for the variant's name only, as
    /// serde applies it; a variant's own `rename` wins.
    pub(crate) fn read(
        derive: Derive,
        variant: &Variant,
        tagged: bool,
        rule: Option<RenameRule>,
    ) -> syn::Result<VariantPlan> {
        let label = derive.spelled();
        let mut rename = None;
        let mut other_span = None;
        let mut explicit_title = None;
        let mut explicit_description = None;

        for attr in &variant.attrs {
            if attr.path().is_ident("map") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        rename = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("other") {
                        other_span = Some(meta.path.clone());
                        Ok(())
                    } else {
                        Err(meta.error(format!(
                            "unrecognised `#[map(...)]` option on a variant. {label} \
                             knows two: `rename = \"...\"`, and `other` on the one \
                             variant of a unit enum that keeps a name it does not know. \
                             A variant cannot be skipped: a value of that variant would \
                             have no way to be written."
                        )))
                    }
                })?;
            } else if attr.path().is_ident("schema") {
                attr.parse_nested_meta(|meta| {
                    if tagged && meta.path.is_ident("title") {
                        explicit_title = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if tagged && meta.path.is_ident("description") {
                        explicit_description = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if !tagged && meta.path.is_ident("label") {
                        explicit_title = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if tagged {
                        Err(meta.error(format!(
                            "unrecognised `#[schema(...)]` option on a variant. {label} \
                             knows `title` and `description`: a variant is an arm, which \
                             is a subschema and takes JSON Schema's own keywords. The \
                             other options describe fields, which go on its fields."
                        )))
                    } else {
                        Err(meta.error(format!(
                            "unrecognised `#[schema(...)]` option on a variant. {label} \
                             knows `label`: a unit variant is one choice among several, \
                             its text is written to `x-enum-labels`, and `label` is that \
                             key's own word. An arm of a tagged enum takes `title`."
                        )))
                    }
                })?;
            }
        }

        let other = match other_span {
            None => None,
            Some(span) if tagged => {
                return Err(syn::Error::new_spanned(
                    span,
                    "`other` is for an enum of unit variants, where a name nobody \
                     declared is still a name. An unknown variant of a tagged enum \
                     would carry fields nobody declared, and nothing here could \
                     keep them.",
                ));
            }
            Some(span) if rename.is_some() || explicit_title.is_some() => {
                return Err(syn::Error::new_spanned(
                    span,
                    "the `other` variant has no name of its own: it is stored as \
                     whatever text it holds, so there is nothing to rename or to \
                     label. Remove `rename` and `label`.",
                ));
            }
            Some(_) => match &variant.fields {
                Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                    Some(unnamed.unnamed[0].ty.clone())
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        &variant.ident,
                        format!(
                            "{label} needs the `#[map(other)]` variant to hold the \
                             text it did not recognise: exactly one unnamed field, as \
                             `Other(String)`."
                        ),
                    ));
                }
            },
        };

        let doc = doc_comment(&variant.attrs);
        let (title_text, description_text) = if other.is_some() {
            (None, None)
        } else if tagged {
            (explicit_title, explicit_description.or(doc))
        } else {
            (explicit_title.or(doc), None)
        };

        let newtype = match &variant.fields {
            Fields::Unnamed(unnamed) if tagged && unnamed.unnamed.len() == 1 => {
                if let Some(attr) = unnamed.unnamed[0]
                    .attrs
                    .iter()
                    .find(|a| a.path().is_ident("map") || a.path().is_ident("schema"))
                {
                    return Err(syn::Error::new_spanned(
                        attr,
                        format!(
                            "{label} takes no `#[map(..)]` or `#[schema(..)]` on a newtype \
                             variant's field: its keys are its type's, written beside the tag."
                        ),
                    ));
                }
                Some(unnamed.unnamed[0].ty.clone())
            }
            _ => None,
        };
        let fields = match &variant.fields {
            Fields::Unit => Vec::new(),
            Fields::Unnamed(_) if other.is_some() || newtype.is_some() => Vec::new(),
            Fields::Named(named) => {
                let mut plan = Vec::new();
                for field in &named.named {
                    plan.push(FieldPlan::read(derive, field, None)?);
                }
                reject_duplicate_keys(&plan)?;
                plan
            }
            Fields::Unnamed(unnamed) => {
                return Err(syn::Error::new_spanned(
                    unnamed,
                    format!(
                        "{label} needs a variant's fields to have names: they are stored \
                         beside the tag under their own keys, and a tuple variant has no \
                         names to key them by. Give the fields names, or hold one type \
                         whose map is written beside the tag: `Name(T)`."
                    ),
                ));
            }
        };

        Ok(VariantPlan {
            stored: rename.unwrap_or_else(|| {
                let name = variant.ident.unraw().to_string();
                rule.map_or(name.clone(), |rule| rule.variant(&name))
            }),
            unit: matches!(variant.fields, Fields::Unit),
            ident: variant.ident.clone(),
            fields,
            title: title_text,
            description: description_text,
            other,
            newtype,
        })
    }
}

fn expand_enum(derive: Derive, ast: &DeriveInput, data: &DataEnum) -> syn::Result<TokenStream> {
    let label = derive.spelled();
    let name = &ast.ident;

    if data.variants.is_empty() {
        return Err(syn::Error::new_spanned(
            name,
            format!(
                "{label} needs at least one variant. An enum with none has no value to \
                 store and none to read back, and its schema would accept nothing."
            ),
        ));
    }

    let ContainerAttrs {
        tag,
        krate,
        rename_all,
        untagged,
        ..
    } = ContainerAttrs::read(derive, &ast.attrs, true)?;
    let head = impl_head(derive, name, &ast.generics);
    let reroot = |tokens| crate::krate::reroot(tokens, krate.as_ref());

    if untagged {
        if tag.is_some() {
            return Err(syn::Error::new_spanned(
                name,
                format!(
                    "{label} takes `tag` or `untagged`, not both: a tag stores each \
                     variant's name under a key, and an untagged enum stores no name, \
                     telling its variants apart by the kind of value each writes."
                ),
            ));
        }
        let generic = ast.generics.type_params().next().is_some();
        return crate::untagged::expand(derive, name, &head, data, rename_all, generic).map(reroot);
    }

    // Before reading any variant, so the message is about the decision
    // that is missing rather than about whichever variant came first.
    if tag.is_none()
        && let Some(carrying) = data
            .variants
            .iter()
            .find(|v| !matches!(v.fields, Fields::Unit) && !says_other(v))
    {
        return Err(syn::Error::new_spanned(
            &carrying.ident,
            format!(
                "{label} needs `#[map(tag = \"...\")]` on an enum whose variants carry \
                 fields. Two variants describe two different maps, and the key that \
                 tells them apart (\"type\", \"kind\", a field name) is a wire-format \
                 decision this crate deliberately does not make for you. Name it, and \
                 each value is a map holding the variant's name under that key beside \
                 its fields."
            ),
        ));
    }

    let mut variants = Vec::new();
    for variant in &data.variants {
        variants.push(VariantPlan::read(
            derive,
            variant,
            tag.is_some(),
            rename_all,
        )?);
    }
    reject_duplicate_variants(&variants)?;
    if let Some(second) = variants.iter().filter(|v| v.other.is_some()).nth(1) {
        return Err(syn::Error::new(
            second.ident.span(),
            "two variants are `#[map(other)]`. A name no variant declares could be \
             read as either, so only one can be the catch-all.",
        ));
    }

    let Some(tag) = tag else {
        return Ok(reroot(match derive {
            Derive::ToValue => emit_choice_to(&head, &variants),
            Derive::FromValue => emit_choice_from(&head, &variants),
            Derive::Schema => emit_choice_schema(&head, &variants),
        }));
    };
    reject_fields_on_the_tag(&tag, &variants)?;
    Ok(reroot(match derive {
        Derive::ToValue => emit_arm_to(&head, &tag, &variants),
        Derive::FromValue => emit_arm_from(&head, &tag, &variants),
        Derive::Schema => emit_arm_schema(&head, &tag, &variants),
    }))
}

/// Whether a variant carries `#[map(other)]`, before its plan is read.
/// Lenient: a malformed attribute is reported by the plan, not here.
pub(crate) fn says_other(variant: &Variant) -> bool {
    let mut found = false;
    for attr in &variant.attrs {
        if attr.path().is_ident("map") {
            let _ = attr.parse_nested_meta(|meta| {
                found |= meta.path.is_ident("other");
                if meta.input.peek(syn::Token![=]) {
                    meta.value()?.parse::<syn::Expr>()?;
                }
                Ok(())
            });
        }
    }
    found
}

/// Two variants stored as one name could not be told apart on the way
/// back, so neither could be read. The catch-all has no stored name.
fn reject_duplicate_variants(variants: &[VariantPlan]) -> syn::Result<()> {
    for (i, variant) in variants.iter().enumerate() {
        if variant.other.is_some() {
            continue;
        }
        if let Some(earlier) = variants[..i]
            .iter()
            .find(|v| v.other.is_none() && v.stored == variant.stored)
        {
            return Err(syn::Error::new(
                variant.ident.span(),
                format!(
                    "two variants are stored as \"{}\": `{}` and `{}`. A value reading \
                     \"{}\" could be either, so neither could be read back. Rename one \
                     with `#[map(rename = \"...\")]`.",
                    variant.stored, earlier.ident, variant.ident, variant.stored
                ),
            ));
        }
    }
    Ok(())
}

/// A field stored under the tag would overwrite the variant's name, and
/// the value could no longer say which variant it is.
fn reject_fields_on_the_tag(tag: &str, variants: &[VariantPlan]) -> syn::Result<()> {
    for variant in variants {
        if let Some(field) = variant.fields.iter().find(|f| !f.skip && f.key == tag) {
            return Err(syn::Error::new(
                field.ident.span(),
                format!(
                    "the field `{}` of `{}` is stored under \"{tag}\", which is the tag. \
                     It would overwrite the variant's name, and the value could no longer \
                     say which variant it is. Rename the field with \
                     `#[map(rename = \"...\")]`, or choose another tag.",
                    field.ident, variant.ident
                ),
            ));
        }
    }
    Ok(())
}

/// What a reader expected, phrased the way `schema::validate` phrases it.
///
/// Names the alternatives and never the value that was given: a value may
/// be a secret, and the caller still holds it.
fn one_of(variants: &[VariantPlan]) -> String {
    let names: Vec<&str> = variants.iter().map(|v| v.stored.as_str()).collect();
    format!("one of {}", names.join(", "))
}

// --- an enum of unit variants: a choice --------------------------------

fn emit_choice_to(head: &TokenStream, variants: &[VariantPlan]) -> TokenStream {
    let arms = variants.iter().map(|v| {
        let ident = &v.ident;
        let stored = &v.stored;
        if v.other.is_some() {
            // Written as the text it holds, which is what it was read from.
            return quote! {
                Self::#ident(ref __text) => ::core::convert::AsRef::<str>::as_ref(__text),
            };
        }
        quote! { Self::#ident => #stored, }
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
                // `*self`: every pattern is a unit variant or binds by
                // `ref`, so nothing is moved out of the borrow.
                ::core::result::Result::Ok(::guatiao::Value::from(
                    ::guatiao::Text::new_in(__alloc, match *self {
                        #(#arms)*
                    })?,
                ))
            }
        }
    }
}

fn emit_choice_from(head: &TokenStream, variants: &[VariantPlan]) -> TokenStream {
    let arms = variants.iter().filter(|v| v.other.is_none()).map(|v| {
        let ident = &v.ident;
        let stored = &v.stored;
        quote! { #stored => ::core::result::Result::Ok(Self::#ident), }
    });
    // A name no variant declares: kept by the catch-all when there is
    // one, refused naming the alternatives when there is not.
    let unknown = match variants.iter().find(|v| v.other.is_some()) {
        Some(v) => {
            let ident = &v.ident;
            let ty = &v.other;
            quote! {
                __other => ::core::result::Result::Ok(Self::#ident(
                    <#ty as ::core::convert::From<&str>>::from(__other),
                )),
            }
        }
        None => {
            let expected = one_of(variants);
            quote! {
                _ => ::core::result::Result::Err(
                    ::guatiao::MapError::bad_value(#expected),
                ),
            }
        }
    };
    quote! {
        #head {
            const KINDS: ::guatiao::Kinds = ::guatiao::Kinds::STRING;

            fn from_value(
                __value: &::guatiao::Value,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                match <&str as ::core::convert::TryFrom<&::guatiao::Value>>::try_from(
                    __value,
                )? {
                    #(#arms)*
                    #unknown
                }
            }
        }
    }
}

fn emit_choice_schema(head: &TokenStream, variants: &[VariantPlan]) -> TokenStream {
    // With a catch-all the names are offered, not required: any text is
    // a value of the type, so the kind must accept any text.
    let build = if variants.iter().any(|v| v.other.is_some()) {
        quote!(open_enumeration_in)
    } else {
        quote!(enumeration_in)
    };
    let rows = variants.iter().filter(|v| v.other.is_none()).map(|v| {
        let stored = &v.stored;
        // No label is written as empty, and the builder leaves an empty
        // one off: a reader shows the value when there is no label.
        let label = v.title.as_deref().unwrap_or("");
        quote! { (#stored, #label), }
    });
    quote! {
        #head {
            fn kind(
                __alloc: ::guatiao::Alloc,
            ) -> ::guatiao::schema::KindBuilder {
                ::guatiao::schema::KindBuilder::#build(__alloc, &[
                    #(#rows)*
                ])
            }
        }
    }
}

// --- an enum with a tag: a variant --------------------------------------

fn emit_arm_to(head: &TokenStream, tag: &str, variants: &[VariantPlan]) -> TokenStream {
    let arms = variants.iter().map(|v| {
        let ident = &v.ident;
        let stored = &v.stored;
        // Bound to generated names, never to the user's field names: a
        // field called `__map` or `__alloc` would otherwise shadow the
        // expansion's own locals. A struct never has this problem, because
        // it reaches its fields through `self.`.
        let bound: Vec<(&FieldPlan, Ident)> = v
            .fields
            .iter()
            .filter(|f| !f.skip)
            .enumerate()
            .map(|(i, f)| (f, format_ident!("__f{i}")))
            .collect();
        if let Some(ty) = &v.newtype {
            return quote! {
                Self::#ident(__f0) => {
                    ::guatiao::Map::set_in(
                        &mut __map,
                        #tag,
                        ::guatiao::Text::new_in(__alloc, #stored)?,
                        __alloc,
                    )?;
                    ::guatiao::__private::flatten_into(
                        &mut __map,
                        <#ty as ::guatiao::ToValue>::to_value_in(__f0, __alloc)?,
                        __alloc,
                    )?;
                }
            };
        }
        let pattern = if v.unit {
            quote! { Self::#ident }
        } else {
            let members = bound.iter().map(|(field, binding)| {
                let member = &field.ident;
                quote! { #member: #binding }
            });
            quote! { Self::#ident { #(#members,)* .. } }
        };
        let sets = bound
            .iter()
            .map(|(field, binding)| set_field(field, &quote! { #binding }));
        quote! {
            #pattern => {
                ::guatiao::Map::set_in(
                    &mut __map,
                    #tag,
                    ::guatiao::Text::new_in(__alloc, #stored)?,
                    __alloc,
                )?;
                #(#sets)*
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
                let mut __map = ::guatiao::Map::new_in(__alloc);
                // The tag goes in first, so the value reads as the variant
                // it is before its payload -- and re-emits byte-stable,
                // because a map is insertion-ordered by contract.
                match self {
                    #(#arms)*
                }
                ::core::result::Result::Ok(::guatiao::Value::from(__map))
            }
        }
    }
}

fn emit_arm_from(head: &TokenStream, tag: &str, variants: &[VariantPlan]) -> TokenStream {
    let arms = variants.iter().map(|v| {
        let ident = &v.ident;
        let stored = &v.stored;
        if let Some(ty) = &v.newtype {
            quote! {
                #stored => ::core::result::Result::Ok(Self::#ident(
                    <#ty as ::guatiao::FromValue>::from_value(__value)?,
                )),
            }
        } else if v.unit {
            quote! { #stored => ::core::result::Result::Ok(Self::#ident), }
        } else {
            let reads = v.fields.iter().map(read_field);
            quote! { #stored => ::core::result::Result::Ok(Self::#ident { #(#reads)* }), }
        }
    });
    let expected = one_of(variants);
    quote! {
        #head {
            const KINDS: ::guatiao::Kinds = ::guatiao::Kinds::MAP;

            fn from_value(
                __value: &::guatiao::Value,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                let __map = <&::guatiao::Map as ::core::convert::TryFrom<
                    &::guatiao::Value,
                >>::try_from(__value)?;
                let __tag = <&str as ::core::convert::TryFrom<&::guatiao::Value>>::try_from(
                    ::guatiao::Map::required(__map, #tag)?,
                )
                .map_err(|__e| ::guatiao::MapError::under(__e, #tag))?;
                // A key the chosen variant does not declare is not refused
                // here, exactly as a struct's reader does not refuse one:
                // reading is lenient and `schema::validate` is the strict
                // check.
                match __tag {
                    #(#arms)*
                    _ => ::core::result::Result::Err(::guatiao::MapError::under(
                        ::guatiao::MapError::bad_value(#expected),
                        #tag,
                    )),
                }
            }
        }
    }
}

/// An arm with every field `ty`'s kind declares added to it.
fn flattened_into_arm(arm: &TokenStream, ty: &Type) -> TokenStream {
    quote! {
        {
            let mut __arm = #arm;
            for __field in ::guatiao::__private::fields_of(<#ty as ::guatiao::Schema>::kind(__alloc)) {
                __arm = __arm.field(__field);
            }
            __arm
        }
    }
}

fn emit_arm_schema(head: &TokenStream, tag: &str, variants: &[VariantPlan]) -> TokenStream {
    let arms = variants.iter().map(|v| {
        let stored = &v.stored;
        let title = v.title.as_deref().unwrap_or("");
        let mut built = quote! {
            ::guatiao::schema::ArmBuilder::new_in(__alloc, #stored, #title)
        };
        if let Some(description) = &v.description {
            built = quote! {
                ::guatiao::schema::ArmBuilder::description(#built, #description)
            };
        }
        // The same builder a struct's fields go through, so `#[map]` and
        // `#[schema]` on a variant's field mean what they mean on a
        // struct's.
        for field in v.fields.iter().filter(|f| !f.skip) {
            if field.flatten {
                built = flattened_into_arm(&built, &field.ty);
                continue;
            }
            let field = field_builder(field);
            built = quote! { #built.field(#field) };
        }
        if let Some(ty) = &v.newtype {
            built = flattened_into_arm(&built, ty);
        }
        quote! { __arms.push(#built); }
    });
    quote! {
        #head {
            fn kind(
                __alloc: ::guatiao::Alloc,
            ) -> ::guatiao::schema::KindBuilder {
                let mut __arms = ::guatiao::__private::alloc::vec::Vec::new();
                #(#arms)*
                ::guatiao::schema::KindBuilder::variant_in(__alloc, #tag, __arms)
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
        // A position has no key to rename, default or describe.
        let positional = to(quote! { struct S(#[map(rename = "a")] u8, u8); });
        assert!(positional.contains("a position has no key"), "{positional}");
        assert!(positional.contains("#[derive(ToValue)]"), "{positional}");

        // A data-carrying enum with no tag. A UNIT enum is accepted, so
        // the refusal is about the one decision this crate will not make.
        let enumeration = from(quote! { enum E { A { x: u8 }, B } });
        assert!(enumeration.contains("compile_error"), "{enumeration}");
        assert!(enumeration.contains("two different maps"), "{enumeration}");
        assert!(
            enumeration.contains("#[map(tag = "),
            "the refusal names the way out: {enumeration}"
        );
        // It must name the derive the user wrote, not the other one.
        assert!(
            enumeration.contains("#[derive(FromValue)]"),
            "{enumeration}"
        );
        assert!(!enumeration.contains("ToValue"), "{enumeration}");

        let union = to(quote! { union U { a: u8, b: u8 } });
        assert!(union.contains("which arm is live"), "{union}");

        let transparent = to(quote! { #[map(transparent)] struct S { a: u8, b: u8 } });
        assert!(transparent.contains("exactly one field"), "{transparent}");
        let deny = from(quote! { #[map(deny_unknown_fields)] struct S(u8); });
        assert!(deny.contains("nothing else has keys"), "{deny}");

        // A container attribute on a STRUCT. Ignored, `tag` would say
        // nothing and a typo would change the wire shape, so both are
        // refused where they were written.
        let tagged_struct = to(quote! { #[map(tag = "kind")] struct S { a: u8 } });
        assert!(tagged_struct.contains("compile_error"), "{tagged_struct}");
        assert!(
            tagged_struct.contains("no variants to tell apart"),
            "{tagged_struct}"
        );

        let bogus_struct = to(quote! { #[map(bogus = "x")] struct S { a: u8 } });
        assert!(bogus_struct.contains("unrecognised"), "{bogus_struct}");
        assert!(
            bogus_struct.contains("go on a field"),
            "the message says where those options belong: {bogus_struct}"
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
        assert!(unknown.contains("default = <expr>"), "{unknown}");

        // A default that could not mean anything, each said where it is.
        let skipped = from(quote! {
            struct S { #[map(skip, default)] a: u8 }
        });
        assert!(
            skipped.contains("`skip` and `default` together"),
            "{skipped}"
        );
        let optional = from(quote! {
            struct S { #[map(default)] a: Option<u8> }
        });
        assert!(optional.contains("already optional"), "{optional}");
        let twice = from(quote! {
            struct S { #[map(default = 1)] #[schema(default = 1)] a: u8 }
        });
        assert!(twice.contains("says it once"), "{twice}");
        // The catch-all, in each place it cannot go.
        let tagged = from(quote! {
            #[map(tag = "t")] enum E { A, #[map(other)] B(String) }
        });
        assert!(tagged.contains("fields nobody declared"), "{tagged}");
        let unit = from(quote! { enum E { A, #[map(other)] B } });
        assert!(unit.contains("exactly one unnamed field"), "{unit}");
        let named = from(quote! {
            enum E { A, #[map(other, rename = "b")] B(String) }
        });
        assert!(named.contains("no name of its own"), "{named}");
        let two = from(quote! {
            enum E { #[map(other)] A(String), #[map(other)] B(String) }
        });
        assert!(two.contains("only one can be the catch-all"), "{two}");
        // A tuple variant that is not the catch-all is still a missing tag.
        let untagged = from(quote! { enum E { A, B(String) } });
        assert!(untagged.contains("#[map(tag = "), "{untagged}");

        let repeated = from(quote! {
            struct S { #[map(default, default = 1)] a: u8 }
        });
        assert!(repeated.contains("given twice"), "{repeated}");

        // Silent overwrite, refused at compile time.
        let clash = to(quote! {
            struct S { a: u8, #[map(rename = "a")] b: u8 }
        });
        assert!(clash.contains("two fields map to the key"), "{clash}");
        assert!(clash.contains("silently replace"), "{clash}");
    }

    /// With `#[map(crate = <path>)]` nothing is rooted at `::guatiao`, on
    /// any shape under any of the three derives.
    #[test]
    fn a_crate_path_reroots_every_generated_path() {
        let shapes = [
            quote! {
                #[map(crate = ::kinds::guatiao)]
                struct S {
                    #[schema(title = "A", description = "h", section = "s", order = 2,
                        advanced, sensitive, read_only, deprecated, examples(1, 2),
                        min_length = 1, pattern = "x", format = "f", multiple_of = 2,
                        min_items = 1, default = 5900)]
                    a: u8,
                    b: Option<u8>,
                    #[map(skip)] c: u8,
                    #[map(default)] d: u8,
                    #[map(default = 7)] e: u8,
                }
            },
            quote! {
                #[map(crate = ::kinds::guatiao)]
                enum C { A, #[map(rename = "b")] B }
            },
            quote! {
                #[map(crate = ::kinds::guatiao)]
                enum O { A, #[map(other)] X(String) }
            },
            quote! {
                #[map(tag = "t", crate = ::kinds::guatiao)]
                enum T { A, B { x: u8, y: Option<u8>, #[map(skip)] z: u8, #[map(default)] w: u8 } }
            },
        ];
        for shape in shapes {
            for out in [
                to(shape.clone()),
                from(shape.clone()),
                schema(shape.clone()),
            ] {
                assert!(!out.contains("compile_error"), "{out}");
                assert!(out.contains(":: kinds :: guatiao ::"), "{out}");
                let rest = out.replace(":: kinds :: guatiao", "");
                assert!(
                    !rest.contains(":: guatiao"),
                    "a path is still rooted at `::guatiao`: {rest}"
                );
            }
        }
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
    fn as_text_stands_apart_from_with_and_transparent() {
        let both = to(quote! { struct S { #[map(as_text, with = m)] a: V } });
        assert!(both.contains("Keep one"), "{both}");
        let flat = to(quote! { struct S { #[map(as_text, flatten)] a: V } });
        assert!(flat.contains("stands alone"), "{flat}");
        let transparent = to(quote! { #[map(transparent)] struct S { #[map(as_text)] a: V } });
        assert!(transparent.contains("no key of its own"), "{transparent}");
        // An `Option` is accepted: `None` omits the key, a null reads `None`.
        let optional = from(quote! { struct S { #[map(as_text)] a: Option<V> } });
        assert!(optional.contains("from_text_or_null"), "{optional}");
    }

    #[test]
    fn every_path_the_expansion_emits_is_absolute() {
        // Every field shape at once, so the scan below sees every path
        // the expansion is capable of emitting -- a skipped field is the
        // only thing that reaches `core::default`.
        // Every presentation attribute is here too, because each one is a
        // different builder call in the `Schema` expansion and the scan
        // below only sees what was emitted. The default is a LITERAL: a
        // default expression is the user's own tokens, so one naming a
        // path would be scanned as though the expansion had emitted it.
        let declaration = quote! {
            struct S {
                #[schema(title = "A", description = "h", section = "s", order = 2, advanced, sensitive, default = 5900)]
                a: u8,
                b: ::core::option::Option<u8>,
                #[map(skip)] c: u8,
                #[map(default)] d: u8,
                #[map(default = 7)] e: u8,
                #[map(as_text)] f: Version,
                #[map(as_text, default)] g: Version,
                #[map(as_text)] h: ::core::option::Option<Version>,
            }
        };
        // And both enum shapes, which reach different helpers: a choice
        // goes through the `&str` conversion, an arm through all four.
        let choice = quote! {
            enum C { A, #[map(rename = "b")] B }
        };
        let open = quote! {
            enum O { A, #[map(other)] X(String) }
        };
        let tagged = quote! {
            #[map(tag = "t")]
            enum T {
                A,
                B { x: u8, y: ::core::option::Option<u8>, #[map(skip)] z: u8, #[map(default)] w: u8 },
            }
        };
        // All THREE derives on all three shapes. `Schema` is not optional
        // here: it is the emitter with the most paths in it, and a scan
        // that skipped it would have proved nothing about the builders.
        let positional = quote! { struct P(u8, ::core::option::Option<u8>); };
        let newtype = quote! { struct N(u8); };
        let unit = quote! { struct U; };
        let flat = quote! {
            #[map(tag = "t")]
            enum F { A(Inner), B { #[map(flatten)] x: Inner } }
        };
        // Told apart by kind: a choice, a record and a held type, whose
        // kinds the reader asserts in a `const`.
        let untagged = quote! {
            #[map(untagged)]
            enum W { A, B { x: u8, #[map(skip)] z: u8 }, C(::core::option::Option<u8>) }
        };
        let output = to(positional.clone())
            + &from(positional.clone())
            + &schema(positional)
            + &to(newtype.clone())
            + &from(newtype.clone())
            + &schema(newtype)
            + &to(unit.clone())
            + &from(unit.clone())
            + &schema(unit)
            + &to(flat.clone())
            + &from(flat.clone())
            + &schema(flat)
            + &to(untagged.clone())
            + &from(untagged.clone())
            + &schema(untagged)
            + &to(declaration.clone())
            + &from(declaration.clone())
            + &schema(declaration)
            + &to(choice.clone())
            + &from(choice.clone())
            + &schema(choice)
            + &to(open.clone())
            + &from(open.clone())
            + &schema(open)
            + &to(tagged.clone())
            + &from(tagged.clone())
            + &schema(tagged);

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
            "Schema",
            "Kinds",
            "Value",
            "Map",
            "Vec",
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
            // The crates generated code reaches into, and the modules on
            // the way to their items. `std` is here for what needs it:
            // `::std::sync::OnceLock`. Everything `alloc` holds is reached
            // as `::guatiao::__private::alloc::`, so the expansion needs no
            // `extern crate alloc` at the call site.
            "guatiao",
            "__private",
            "alloc",
            "core",
            "std",
            "default",
            "option",
            "result",
            "vec",
            // The `guatiao` module generated code names: `schema`, for
            // the builders. `convert` is `core`'s, for `TryFrom`.
            "convert",
            "schema",
            // Types and traits from `guatiao`.
            "Alloc",
            "Map",
            "Text",
            "ValueError",
            "FromValue",
            "MapError",
            "Value",
            "ToValue",
            "Schema",
            "ArmBuilder",
            "FieldBuilder",
            "KindBuilder",
            "Number",
            // A tuple struct is a list.
            "List",
            // What a reader accepts, for an enum told apart by kind.
            "Kinds",
            // The door the presentation keys go through. Their names are
            // `guatiao-intake`'s and are emitted as literals, so nothing
            // here names that crate -- which is the point: a crate
            // deriving a schema need not depend on it.
            "Extras",
            // Items from `core` and `std`.
            "Default",
            "Option",
            "Result",
            "TryFrom",
            "Vec",
            // How a catch-all variant's field is written as text.
            "AsRef",
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
        // loop above would pass while checking nothing at all. `schema`
        // and `__private` are named because they appear ONLY in the `Schema`
        // expansion -- this is what says that expansion was scanned, which
        // for a while it was not.
        for expected in ["guatiao", "core", "schema", "__private"] {
            assert!(
                segments.contains(&expected),
                "the segment scan never saw `{expected}`, so it proved less than it \
                 looks: {segments:?}"
            );
        }
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
            from_side.contains("Map :: required"),
            "a required field reports a missing key: {from_side}"
        );
        assert!(
            from_side.contains(":: guatiao :: Map as :: core :: convert :: TryFrom"),
            "a value that is not a map is rejected as that, not as every field \
             missing: {from_side}"
        );
    }

    /// The shapes beside a named struct, each as serde writes it: a
    /// newtype is its field, a tuple struct a list, a unit struct a null,
    /// a generic struct bounded by the trait for each type parameter.
    #[test]
    fn every_struct_shape_expands() {
        let newtype = to(quote! { struct Port(u16); });
        assert!(
            newtype.contains("< u16 as :: guatiao :: ToValue > :: to_value_in (& self . 0"),
            "{newtype}"
        );
        let transparent =
            from(quote! { #[map(transparent)] struct P { port: u16, #[map(skip)] cache: u8 } });
        assert!(
            transparent
                .contains("port : < u16 as :: guatiao :: FromValue > :: from_value (__value)"),
            "{transparent}"
        );
        assert!(
            transparent.contains("cache : :: core :: default :: Default :: default ()"),
            "{transparent}"
        );
        let tuple = schema(quote! { struct Pair(u8, String); });
        assert!(tuple.contains("tuple_in"), "{tuple}");
        let read = from(quote! { struct Pair(u8, String); });
        assert!(read.contains("a list of exactly 2 items"), "{read}");
        let unit = to(quote! { struct Marker; });
        assert!(unit.contains(":: guatiao :: Value :: null ()"), "{unit}");
        let generic = to(quote! { struct W<'a, T, const N: usize> { a: &'a T, b: [u8; N] } });
        assert!(
            generic.contains("impl < 'a , T , const N : usize > :: guatiao :: ToValue for W < 'a , T , N > where T : :: guatiao :: ToValue"),
            "{generic}"
        );
        let generic = schema(quote! { #[map(tag = "t")] enum E<T> { A { x: T } } });
        assert!(
            generic.contains("where T : :: guatiao :: Schema"),
            "{generic}"
        );
    }

    /// `flatten`, `with`, `skip_serializing_if` and `deny_unknown_fields`
    /// reach the expansion; the combinations that cannot mean anything are
    /// refused by name.
    #[test]
    fn serde_shaped_field_attributes() {
        let decl = quote! {
            #[map(deny_unknown_fields)]
            struct S {
                #[map(with = "secs")] a: Duration,
                #[map(with = minutes)] b: Duration,
                #[map(default, skip_serializing_if = "Vec::is_empty")] c: Vec<u8>,
                d: u8,
            }
        };
        let to_side = to(decl.clone());
        assert!(
            to_side.contains("secs :: to_value_in (& self . a"),
            "{to_side}"
        );
        assert!(to_side.contains("minutes :: to_value_in"), "{to_side}");
        assert!(
            to_side.contains("if ! Vec :: is_empty (& self . c)"),
            "{to_side}"
        );
        let from_side = from(decl.clone());
        assert!(from_side.contains("secs :: from_value"), "{from_side}");
        assert!(from_side.contains("\"a declared field\""), "{from_side}");
        let schema_side = schema(decl);
        assert!(
            schema_side.contains("secs :: kind (__alloc)"),
            "{schema_side}"
        );

        let flat = quote! { struct S { #[map(flatten)] inner: Inner, b: u8 } };
        assert!(to(flat.clone()).contains("__private :: flatten_into"));
        assert!(
            from(flat.clone())
                .contains("inner : < Inner as :: guatiao :: FromValue > :: from_value (__value)")
        );
        assert!(schema(flat).contains("__private :: fields_of"));

        for (decl, expected) in [
            (
                quote! { struct S { #[map(flatten, rename = "x")] a: A } },
                "`flatten` stands alone",
            ),
            (
                quote! { struct S { #[map(flatten)] a: Option<A> } },
                "an `Option` writes a null",
            ),
            (
                quote! { struct S { #[map(with = m)] a: Option<A> } },
                "`with` converts a whole field",
            ),
            (
                quote! { struct S { #[map(skip_serializing_if = "f")] a: u8 } },
                "needs `#[map(default)]`",
            ),
            (
                quote! { struct S { #[map(alias = "b")] a: u8 } },
                "Not `alias`",
            ),
            (
                quote! { #[map(deny_unknown_fields)] struct S { #[map(flatten)] a: A } },
                "beside a `#[map(flatten)]`",
            ),
        ] {
            let out = from(decl);
            assert!(out.contains(expected), "want {expected}: {out}");
        }
        // An `Option` already omits `None`; a predicate may skip more.
        let optional =
            to(quote! { struct S { #[map(skip_serializing_if = "is_zero")] a: Option<u8> } });
        assert!(!optional.contains("compile_error"), "{optional}");
    }

    /// A struct with no fields at all is legal and produces an empty map.
    /// Degenerate, but reachable from a macro that generates structs, and
    /// there is nothing wrong with it.
    #[test]
    fn an_empty_named_struct_is_accepted() {
        let output = to(quote! { struct S {} });
        assert!(!output.contains("compile_error"), "{output}");
        assert!(output.contains("Map :: new_in (__alloc)"), "{output}");
    }

    fn schema(input: TokenStream) -> String {
        rendered(Derive::Schema, input)
    }

    /// The shape keywords chain onto the KIND, the annotations onto the
    /// FIELD, and each is a call by path into `guatiao`'s own builders.
    #[test]
    fn shape_keywords_and_annotations_reach_the_builders() {
        let out = schema(quote! {
            struct S {
                #[schema(min_length = 2, max_length = 8, pattern = "^[a-z]+$", format = "email")]
                user: String,
                #[schema(multiple_of = 5, read_only)]
                port: i64,
                #[schema(multiple_of = 0.25, deprecated)]
                ratio: f64,
                #[schema(min_items = 1, max_items = 3, examples("a", "b"))]
                tags: Vec<String>,
            }
        });
        for expected in [
            "KindBuilder :: min_length",
            "KindBuilder :: max_length",
            "KindBuilder :: pattern",
            "KindBuilder :: format",
            "KindBuilder :: multiple_of",
            "KindBuilder :: min_items",
            "KindBuilder :: max_items",
            "FieldBuilder :: read_only",
            "FieldBuilder :: deprecated",
            "vocab :: EXAMPLES",
        ] {
            assert!(out.contains(expected), "missing {expected}: {out}");
        }
        assert!(!out.contains("compile_error"), "{out}");
    }

    #[test]
    fn a_step_that_divides_nothing_is_refused() {
        for bad in [quote! { 0 }, quote! { 0.0 }, quote! { -5 }] {
            let out = schema(quote! { struct S { #[schema(multiple_of = #bad)] n: i64 } });
            assert!(out.contains("compile_error"), "{bad}: {out}");
        }
        let out = schema(quote! { struct S { #[schema(multiple_of = "5")] n: i64 } });
        assert!(out.contains("a number"), "{out}");
    }

    /// A unit enum is a choice: its value is the variant's name, and no
    /// wire-format decision was needed to say so.
    #[test]
    fn a_unit_enum_is_a_choice() {
        let decl = quote! {
            enum Level {
                /// Nothing at all.
                Off,
                #[map(rename = "warn")]
                #[schema(label = "Warnings only")]
                Warning,
                On,
            }
        };

        let to_side = to(decl.clone());
        assert!(!to_side.contains("compile_error"), "{to_side}");
        assert!(to_side.contains("Text :: new_in"), "{to_side}");
        assert!(
            to_side.contains("Self :: Warning => \"warn\""),
            "a rename reaches the stored spelling: {to_side}"
        );

        let from_side = from(decl.clone());
        assert!(
            from_side.contains("& str as :: core :: convert :: TryFrom"),
            "{from_side}"
        );
        assert!(
            from_side.contains("\"one of Off, warn, On\""),
            "an unknown spelling names the alternatives: {from_side}"
        );

        let schema_side = schema(decl);
        assert!(schema_side.contains("enumeration_in"), "{schema_side}");
        assert!(
            schema_side.contains("(\"Off\" , \"Nothing at all.\")"),
            "a choice has only a label, so its doc comment IS the label: {schema_side}"
        );
        assert!(
            schema_side.contains("(\"warn\" , \"Warnings only\")"),
            "an explicit label wins: {schema_side}"
        );
        assert!(
            schema_side.contains("(\"On\" , \"\")"),
            "no label at all is written empty, and the builder leaves it off: {schema_side}"
        );
    }

    /// A tagged enum is a variant: a map holding the name under the tag,
    /// then the variant's own fields.
    #[test]
    fn a_tagged_enum_is_a_variant() {
        let decl = quote! {
            #[map(tag = "auth")]
            enum Auth {
                /// The ambient credential.
                Ambient,
                #[map(rename = "userpass")]
                UserPass { username: String, password: Option<String>, #[map(skip)] cache: u8 },
            }
        };

        let to_side = to(decl.clone());
        assert!(!to_side.contains("compile_error"), "{to_side}");
        assert!(
            to_side.contains("username : __f0"),
            "fields are bound to generated names, never the user's: {to_side}"
        );
        assert!(
            !to_side.contains("cache :"),
            "a skipped field is not bound: {to_side}"
        );

        let from_side = from(decl.clone());
        assert!(
            from_side.contains("Map :: required (__map , \"auth\")"),
            "{from_side}"
        );
        assert!(
            from_side.contains("cache : :: core :: default :: Default :: default ()"),
            "a skipped field is defaulted, as in a struct: {from_side}"
        );

        let schema_side = schema(decl);
        assert!(
            schema_side.contains("variant_in (__alloc , \"auth\""),
            "{schema_side}"
        );
        assert!(
            schema_side.contains("ArmBuilder :: description"),
            "an arm has a description as well as a title, so its doc comment is the \
             description: {schema_side}"
        );
        assert!(
            schema_side.contains("FieldBuilder :: new_in (__alloc , \"username\""),
            "an arm's fields go through the same builder a struct's do: {schema_side}"
        );
    }

    /// The enum-specific refusals, each by its message.
    #[test]
    fn enum_mistakes_are_named() {
        let empty = to(quote! { enum Never {} });
        assert!(empty.contains("at least one variant"), "{empty}");

        // One unnamed field is a newtype arm; two have no keys.
        let tuple = to(quote! { #[map(tag = "k")] enum E { A(u8, u8) } });
        assert!(tuple.contains("tuple variant"), "{tuple}");
        assert!(tuple.contains("`Name(T)`"), "{tuple}");
        let newtype = to(quote! { #[map(tag = "k")] enum E { A(#[map(rename = "x")] u8) } });
        assert!(newtype.contains("newtype variant"), "{newtype}");

        let same = to(quote! { enum E { A, #[map(rename = "A")] B } });
        assert!(same.contains("two variants are stored as"), "{same}");

        let on_tag = to(quote! {
            #[map(tag = "kind")]
            enum E { A { #[map(rename = "kind")] k: u8 } }
        });
        assert!(on_tag.contains("which is the tag"), "{on_tag}");

        let typo = to(quote! { #[map(tags = "k")] enum E { A } });
        assert!(
            typo.contains("unrecognised") && typo.contains("tag = "),
            "a misspelled tag is refused, never ignored -- ignoring it would change \
             the wire shape: {typo}"
        );

        let skip = to(quote! { enum E { #[map(skip)] A } });
        assert!(skip.contains("cannot be skipped"), "{skip}");

        // A choice takes `label`, the word `x-enum-labels` uses; `title`
        // is the arm spelling and is refused here, with the difference
        // named rather than left to be guessed at.
        let described = schema(quote! { enum E { #[schema(description = "x")] A } });
        assert!(
            described.contains("`label` is that key's own word"),
            "{described}"
        );
        let titled = schema(quote! { enum E { #[schema(title = "x")] A } });
        assert!(
            titled.contains("An arm of a tagged enum takes `title`"),
            "{titled}"
        );

        let generic = to(quote! { #[map(tag = "t")] enum E<T> { A(T) } });
        assert!(
            generic.contains("where T : :: guatiao :: ToValue"),
            "{generic}"
        );
    }
}
