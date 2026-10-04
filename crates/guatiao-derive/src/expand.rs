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
//! # Three shapes
//!
//! | declaration | value | schema kind |
//! | --- | --- | --- |
//! | struct with named fields | a map, one key per field | an object |
//! | enum of unit variants | the variant's name, as text | a string `enum` |
//! | enum with `#[map(tag = "k")]` | a map: the name under `k`, then the variant's fields | a tagged variant |
//!
//! An enum whose variants carry fields and that names no tag is refused:
//! the key that tells two variants apart is a wire-format decision, and
//! the author makes it rather than this crate.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Attribute, Data, DataEnum, DeriveInput, Field, Fields, FieldsNamed, GenericArgument, Ident,
    LitStr, PathArguments, Type, Variant,
};

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
    let label = derive.spelled();
    match &ast.data {
        Data::Struct(data) => {
            refuse_generics(derive, &ast)?;
            // The same reader the enum path uses, so a container-level
            // `#[map(...)]` is refused on a struct rather than ignored --
            // ignoring one would change the wire shape silently.
            ContainerAttrs::read(derive, &ast.attrs, false)?;
            let named = named_fields(derive, &ast.ident, &data.fields)?;

            let mut plan: Vec<FieldPlan> = Vec::new();
            for field in &named.named {
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
        Data::Enum(data) => {
            refuse_generics(derive, &ast)?;
            expand_enum(derive, &ast, data)
        }
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

/// Generic parameters are refused rather than guessed at.
///
/// A generated impl would have to invent the bound for each parameter
/// (`T: ToValue`? `T: Schema`? both?), and inventing it wrong produces an
/// error inside the expansion -- exactly the class of message this file
/// exists to avoid. Refusing at the declaration costs the user a
/// hand-written impl and tells them so at the point they can act.
fn refuse_generics(derive: Derive, ast: &DeriveInput) -> syn::Result<()> {
    if ast.generics.params.is_empty() {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        &ast.generics,
        format!(
            "{} does not support generic parameters. The bound to place on each \
             parameter cannot be inferred here, and guessing it wrong reports an \
             error inside the expansion rather than on your declaration. Write the \
             impl by hand.",
            derive.spelled()
        ),
    ))
}

/// A struct's named fields, or a message explaining why this particular
/// shape has no map to be.
fn named_fields<'a>(
    derive: Derive,
    ident: &Ident,
    fields: &'a Fields,
) -> syn::Result<&'a FieldsNamed> {
    let label = derive.spelled();
    match fields {
        Fields::Named(named) => Ok(named),
        Fields::Unnamed(unnamed) => Err(syn::Error::new_spanned(
            unnamed,
            format!(
                "{label} needs named fields: a map is keyed by name, and a tuple \
                 struct has no names to key it by. Give the fields names, or write \
                 the impl by hand and choose the keys yourself."
            ),
        )),
        Fields::Unit => Err(syn::Error::new_spanned(
            ident,
            format!(
                "{label} needs named fields, and a unit struct has none. A type \
                 carrying nothing converts to an empty map, which is \
                 `Map::new()` -- no derive required."
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
    /// What an absent key reads as. With one the field is not required.
    default: Option<MapDefault>,
    /// What only the schema reads. Carried on every plan so the three
    /// derives share one parse of one declaration.
    schema: SchemaAttrs,
}

/// `#[map(default)]` or `#[map(default = <expr>)]`.
enum MapDefault {
    /// The field type's own `Default`.
    Trait,
    /// An expression of the field's type.
    Expr(syn::Expr),
}

impl FieldPlan {
    fn read(derive: Derive, field: &Field) -> syn::Result<FieldPlan> {
        let ident = field
            .ident
            .clone()
            .expect("only named fields reach a FieldPlan");
        let attrs = FieldAttrs::read(derive, field)?;
        Ok(FieldPlan {
            key: attrs.rename.unwrap_or_else(|| ident.to_string()),
            inner: option_inner(&field.ty).cloned(),
            skip: attrs.skip,
            default: attrs.default,
            schema: attrs.schema,
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
                        "unrecognised `#[map(...)]` option. {label} knows three: \
                         `rename = \"...\"`, `skip`, and `default` or \
                         `default = <expr>`."
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

// --- one field, three ways ---------------------------------------------
//
// A struct's fields and a tagged variant's fields are the same thing, so
// they are emitted by the same three functions. What differs is only how
// the field is reached: `&self.name` in a struct, a pattern binding in a
// variant.

/// Writes one field into `__map`. `access` borrows the field.
fn set_field(field: &FieldPlan, access: &TokenStream) -> TokenStream {
    let key = &field.key;
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
                    <#inner as ::guatiao::ToValue>::to_value(__value, __alloc)?,
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
                <#ty as ::guatiao::ToValue>::to_value(#access, __alloc)?,
                __alloc,
            )?;
        }
    }
}

/// Reads one field out of the map `__value`, as a struct-literal member.
fn read_field(field: &FieldPlan) -> TokenStream {
    let ident = &field.ident;
    let ty = &field.ty;
    let key = &field.key;
    if field.skip {
        // Nothing here can invent a value, so `Default` is required -- and
        // requiring it at the use site is what makes the error name the
        // user's own type rather than the expansion.
        return quote! { #ident: ::core::default::Default::default(), };
    }
    // `under` is what turns a leaf error into a dotted path: the value
    // being read does not know its own key, so the reader that knows it
    // adds it here, on the way out.
    if let Some(default) = field.default_expr() {
        return quote! {
            #ident: match __map.get(#key) {
                ::core::option::Option::Some(__field) =>
                    <#ty as ::guatiao::FromValue>::from_value(__field)
                        .map_err(|__e| ::guatiao::MapError::under(__e, #key))?,
                ::core::option::Option::None => #default,
            },
        };
    }
    if field.optional() {
        quote! {
            #ident: match __map.get(#key) {
                ::core::option::Option::Some(__field) =>
                    <#ty as ::guatiao::FromValue>::from_value(__field)
                        .map_err(|__e| ::guatiao::MapError::under(__e, #key))?,
                // Absent reads as `None`, exactly as a stored null does.
                // See `guatiao`'s `convert` module.
                ::core::option::Option::None => ::core::option::Option::None,
            },
        }
    } else {
        quote! {
            #ident: <#ty as ::guatiao::FromValue>::from_value(
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
fn field_builder(field: &FieldPlan) -> TokenStream {
    let key = &field.key;
    let ty = &field.ty;
    let a = &field.schema;
    // The shape keywords chain onto the KIND, before it becomes a field.
    // Inherent methods, called by path for the same reason as below.
    let mut kind = quote! { <#ty as ::guatiao::Schema>::kind(__alloc) };
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
                                .to_value(__alloc)
                                .and_then(|__v| __examples.push(__v));
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
    if let Some(default) = field.default_expr() {
        // The value an absent key reads as is the one the schema declares,
        // so a consumer filling a form starts from what the reader uses.
        built = quote! {
            #built.default_checked(
                <#ty as ::guatiao::ToValue>::to_value(&#default, __alloc),
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
                <#ty as ::guatiao::ToValue>::to_value(&(#default), __alloc),
            )
        };
    }
    built
}

// --- structs -----------------------------------------------------------

fn emit_to(name: &Ident, plan: &[FieldPlan]) -> TokenStream {
    let sets = plan.iter().filter(|f| !f.skip).map(|field| {
        let ident = &field.ident;
        set_field(field, &quote! { &self.#ident })
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
                let mut __map = ::guatiao::Map::new_in(__alloc);
                #(#sets)*
                ::core::result::Result::Ok(::guatiao::Value::from(__map))
            }
        }
    }
}

fn emit_from(name: &Ident, plan: &[FieldPlan]) -> TokenStream {
    let reads = plan.iter().map(read_field);

    quote! {
        #[automatically_derived]
        impl ::guatiao::FromValue for #name {
            fn from_value(
                __value: &::guatiao::Value,
            ) -> ::core::result::Result<Self, ::guatiao::MapError> {
                // First, so handing this a string reports THAT rather
                // than reporting every field missing. The error names no
                // key; whoever recursed adds one.
                let __map = <&::guatiao::Map as ::core::convert::TryFrom<
                    &::guatiao::Value,
                >>::try_from(__value)?;
                ::core::result::Result::Ok(#name {
                    #(#reads)*
                })
            }
        }
    }
}

/// The schema, as builder calls.
fn emit_schema(name: &Ident, plan: &[FieldPlan]) -> TokenStream {
    let fields = plan.iter().filter(|f| !f.skip).map(|field| {
        let built = field_builder(field);
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
                if meta.path.is_ident("tag") && is_enum {
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
                         one: `tag = \"...\"`, the key a variant's name is stored under."
                    )))
                } else {
                    Err(meta.error(format!(
                        "unrecognised `#[map(...)]` option on a struct. {label} takes \
                         none here: `rename = \"...\"` and `skip` go on a field, and \
                         `tag` is an enum's."
                    )))
                }
            })?;
        }
        Ok(out)
    }
}

/// One variant, read once and shared by all three derives.
struct VariantPlan {
    ident: Ident,
    /// What is stored for it: the variant's own name, or its
    /// `#[map(rename = "...")]`.
    stored: String,
    /// A unit variant, which is constructed and matched without braces.
    unit: bool,
    /// Named fields, in declaration order. Empty for a unit variant.
    fields: Vec<FieldPlan>,
    /// The text shown for it: an arm's `title`, or a choice's entry in
    /// `x-enum-labels` -- which is why a choice declares it as `label`.
    title: Option<String>,
    /// An arm's `description`. A choice has none.
    description: Option<String>,
    /// `#[map(other)]`: the catch-all of a unit enum, holding a name no
    /// other variant is stored as. The type of its one field.
    other: Option<Type>,
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
    fn read(derive: Derive, variant: &Variant, tagged: bool) -> syn::Result<VariantPlan> {
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

        let fields = match &variant.fields {
            Fields::Unit => Vec::new(),
            Fields::Unnamed(_) if other.is_some() => Vec::new(),
            Fields::Named(named) => {
                let mut plan = Vec::new();
                for field in &named.named {
                    plan.push(FieldPlan::read(derive, field)?);
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
                         names to key them by. Give the fields names."
                    ),
                ));
            }
        };

        Ok(VariantPlan {
            stored: rename.unwrap_or_else(|| variant.ident.to_string()),
            unit: matches!(variant.fields, Fields::Unit),
            ident: variant.ident.clone(),
            fields,
            title: title_text,
            description: description_text,
            other,
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

    let tag = ContainerAttrs::read(derive, &ast.attrs, true)?.tag;

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
        variants.push(VariantPlan::read(derive, variant, tag.is_some())?);
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
        return Ok(match derive {
            Derive::ToValue => emit_choice_to(name, &variants),
            Derive::FromValue => emit_choice_from(name, &variants),
            Derive::Schema => emit_choice_schema(name, &variants),
        });
    };
    reject_fields_on_the_tag(&tag, &variants)?;
    Ok(match derive {
        Derive::ToValue => emit_arm_to(name, &tag, &variants),
        Derive::FromValue => emit_arm_from(name, &tag, &variants),
        Derive::Schema => emit_arm_schema(name, &tag, &variants),
    })
}

/// Whether a variant carries `#[map(other)]`, before its plan is read.
/// Lenient: a malformed attribute is reported by the plan, not here.
fn says_other(variant: &Variant) -> bool {
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

fn emit_choice_to(name: &Ident, variants: &[VariantPlan]) -> TokenStream {
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
        #[automatically_derived]
        impl ::guatiao::ToValue for #name {
            fn to_value(
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

fn emit_choice_from(name: &Ident, variants: &[VariantPlan]) -> TokenStream {
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
        #[automatically_derived]
        impl ::guatiao::FromValue for #name {
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

fn emit_choice_schema(name: &Ident, variants: &[VariantPlan]) -> TokenStream {
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
        #[automatically_derived]
        impl ::guatiao::Schema for #name {
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

fn emit_arm_to(name: &Ident, tag: &str, variants: &[VariantPlan]) -> TokenStream {
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
        #[automatically_derived]
        impl ::guatiao::ToValue for #name {
            fn to_value(
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

fn emit_arm_from(name: &Ident, tag: &str, variants: &[VariantPlan]) -> TokenStream {
    let arms = variants.iter().map(|v| {
        let ident = &v.ident;
        let stored = &v.stored;
        if v.unit {
            quote! { #stored => ::core::result::Result::Ok(Self::#ident), }
        } else {
            let reads = v.fields.iter().map(read_field);
            quote! { #stored => ::core::result::Result::Ok(Self::#ident { #(#reads)* }), }
        }
    });
    let expected = one_of(variants);
    quote! {
        #[automatically_derived]
        impl ::guatiao::FromValue for #name {
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

fn emit_arm_schema(name: &Ident, tag: &str, variants: &[VariantPlan]) -> TokenStream {
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
            let field = field_builder(field);
            built = quote! { #built.field(#field) };
        }
        quote! { __arms.push(#built); }
    });
    quote! {
        #[automatically_derived]
        impl ::guatiao::Schema for #name {
            fn kind(
                __alloc: ::guatiao::Alloc,
            ) -> ::guatiao::schema::KindBuilder {
                let mut __arms = ::std::vec::Vec::new();
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
        let tuple = to(quote! { struct S(u8, u8); });
        assert!(tuple.contains("compile_error"), "{tuple}");
        assert!(tuple.contains("needs named fields"), "{tuple}");
        assert!(tuple.contains("tuple struct"), "{tuple}");
        assert!(tuple.contains("#[derive(ToValue)]"), "{tuple}");

        let unit = to(quote! { struct S; });
        assert!(unit.contains("unit struct has none"), "{unit}");
        assert!(unit.contains("Map::new()"), "{unit}");

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

        let generic = to(quote! { struct S<T> { a: T } });
        assert!(generic.contains("does not support generic"), "{generic}");
        assert!(generic.contains("Write the impl by hand"), "{generic}");

        let lifetime = to(quote! { struct S<'a> { a: &'a str } });
        assert!(
            lifetime.contains("does not support generic"),
            "a lifetime is a generic parameter too: {lifetime}"
        );

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
        let output = to(declaration.clone())
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
            // the way to their items. `std` is here for one call:
            // `::std::vec::Vec::new()`, which the `Schema` emitters use
            // instead of `vec![]` so the expansion names no macro the call
            // site has to resolve. It is the one path that is neither
            // `::guatiao::` nor `::core::`.
            "guatiao",
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
        // and `std` are named because they appear ONLY in the `Schema`
        // expansion -- this is what says that expansion was scanned, which
        // for a while it was not.
        for expected in ["guatiao", "core", "schema", "std"] {
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

        let tuple = to(quote! { #[map(tag = "k")] enum E { A(u8) } });
        assert!(tuple.contains("tuple variant"), "{tuple}");

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

        let generic = to(quote! { enum E<T> { A(T) } });
        assert!(generic.contains("does not support generic"), "{generic}");
    }
}
