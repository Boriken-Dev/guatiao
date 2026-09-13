// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[derive(Form)]`: a type's default screen, declared beside its fields.
//!
//! What the schema derive cannot say — what a section is **called**, which
//! **widget** draws a field, its **placeholder**, and **when** it is shown —
//! is written here and emitted as an `impl guatiao_form::Screen`, which
//! builds the form value through `guatiao_form`'s own builders:
//!
//! ```ignore
//! #[derive(Schema, Form)]
//! #[form(section(id = "net", label = "Network", help = "Where to connect"))]
//! struct Connection {
//!     #[schema(section = "net")]
//!     host: String,
//!     #[form(widget = "password")]
//!     token: String,
//!     verify: bool,
//!     #[form(placeholder = "/etc/ssl/ca.pem", visible_when(field = "verify", equals = true))]
//!     ca: Option<String>,
//!     #[form(nested)]
//!     auth: Auth,               // `Auth` derives `Form`; its hints land under `auth.`
//! }
//! ```
//!
//! On the type: `section(id = "..", label = "..", help = "..")`, repeated,
//! in display order. On a field: `widget = ".."`, `placeholder = ".."`,
//! `visible_when(field = "..", equals = <expr>)`, and `nested`, which says
//! the field's type derives `Form` too and composes its hints under
//! `<key>.`. Keys follow `#[map(rename = "..")]`, so the form names the
//! same paths the schema and the value do; a `#[map(skip)]` field cannot
//! carry hints, since it has no key to name.
//!
//! An enum with fields (`#[map(tag = "..")]`) contributes each arm field
//! under its own key — `owner.member` once composed by an owner's
//! `nested` — and may declare sections; an enum of unit variants is one
//! choice and has no fields to place, so it is refused.
//!
//! The derive is `Form`, after what it builds; the trait is `Screen`,
//! because `guatiao_form::Form` is the builder it uses.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, LitStr, Type};

const LABEL: &str = "#[derive(Form)]";

/// Expands the derive, or emits a compile error at the user's span.
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    match try_expand(input) {
        Ok(tokens) => tokens,
        Err(e) => e.to_compile_error(),
    }
}

/// A section declared on the type.
struct SectionDecl {
    id: String,
    label: Option<String>,
    help: Option<String>,
}

/// What one field says about its screen.
#[derive(Default)]
struct FieldHints {
    widget: Option<String>,
    placeholder: Option<String>,
    visible_when: Option<(String, syn::Expr)>,
    nested: bool,
}

impl FieldHints {
    fn is_empty(&self) -> bool {
        self.widget.is_none()
            && self.placeholder.is_none()
            && self.visible_when.is_none()
            && !self.nested
    }
}

/// One field with a key and, maybe, hints.
struct FieldDecl {
    key: String,
    ty: Type,
    hints: FieldHints,
}

fn try_expand(input: TokenStream) -> syn::Result<TokenStream> {
    let ast: DeriveInput = syn::parse2(input)?;
    if let Some(param) = ast.generics.params.iter().next() {
        return Err(syn::Error::new_spanned(
            param,
            format!("{LABEL} refuses a generic type: a screen is declared for one type"),
        ));
    }
    let sections = read_sections(&ast.attrs)?;
    let fields = match &ast.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => read_fields(named.named.iter())?,
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    format!("{LABEL} needs named fields: a form names fields by key"),
                ));
            }
        },
        Data::Enum(data) => {
            let mut fields = Vec::new();
            let mut any_fields = false;
            for variant in &data.variants {
                if let Some(attr) = variant.attrs.iter().find(|a| a.path().is_ident("form")) {
                    return Err(syn::Error::new_spanned(
                        attr,
                        format!(
                            "{LABEL} takes no `#[form(..)]` on a variant: an arm is a choice, \
                             and hints go on the fields it adds"
                        ),
                    ));
                }
                match &variant.fields {
                    Fields::Unit => {}
                    Fields::Named(named) => {
                        any_fields = true;
                        fields.extend(read_fields(named.named.iter())?);
                    }
                    Fields::Unnamed(unnamed) => {
                        return Err(syn::Error::new_spanned(
                            unnamed,
                            format!("{LABEL} needs a variant's fields to have names"),
                        ));
                    }
                }
            }
            if !any_fields {
                return Err(syn::Error::new_spanned(
                    &ast.ident,
                    format!(
                        "{LABEL} has nothing to place on an enum of unit variants: it is one \
                         choice, drawn by the widget of the field holding it"
                    ),
                ));
            }
            fields
        }
        Data::Union(data) => {
            return Err(syn::Error::new_spanned(
                data.union_token,
                format!("{LABEL} supports structs with named fields and enums"),
            ));
        }
    };
    Ok(emit(&ast.ident, &sections, &fields))
}

fn read_sections(attrs: &[Attribute]) -> syn::Result<Vec<SectionDecl>> {
    let mut sections = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("form") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("section") {
                return Err(meta.error(format!(
                    "unrecognised `#[form(..)]` option on a type. {LABEL} knows \
                     `section(id = \"..\", label = \"..\", help = \"..\")`, repeated in \
                     display order; hints go on the fields"
                )));
            }
            let mut id = None;
            let mut label = None;
            let mut help = None;
            meta.parse_nested_meta(|inner| {
                if inner.path.is_ident("id") {
                    id = Some(inner.value()?.parse::<LitStr>()?.value());
                } else if inner.path.is_ident("label") {
                    label = Some(inner.value()?.parse::<LitStr>()?.value());
                } else if inner.path.is_ident("help") {
                    help = Some(inner.value()?.parse::<LitStr>()?.value());
                } else {
                    return Err(inner.error(format!(
                        "unrecognised section option. {LABEL} knows `id`, `label` and `help`"
                    )));
                }
                Ok(())
            })?;
            let Some(id) = id else {
                return Err(meta.error("a section needs `id = \"..\"`: the id its fields name"));
            };
            sections.push(SectionDecl { id, label, help });
            Ok(())
        })?;
    }
    Ok(sections)
}

fn read_fields<'a>(fields: impl Iterator<Item = &'a syn::Field>) -> syn::Result<Vec<FieldDecl>> {
    let mut out = Vec::new();
    for field in fields {
        let ident = field.ident.as_ref().expect("only named fields reach here");
        let mut key = ident.to_string();
        let mut skip = false;
        let mut hints = FieldHints::default();
        for attr in &field.attrs {
            if attr.path().is_ident("map") {
                // `rename` and `skip` decide the key; anything else is
                // the value derives' business and refused there.
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        key = meta.value()?.parse::<LitStr>()?.value();
                    } else if meta.path.is_ident("skip") {
                        skip = true;
                    } else if meta.input.peek(syn::Token![=]) {
                        let _: syn::Expr = meta.value()?.parse()?;
                    }
                    Ok(())
                })?;
            } else if attr.path().is_ident("form") {
                read_hints(attr, &mut hints)?;
            }
        }
        if skip && !hints.is_empty() {
            return Err(syn::Error::new_spanned(
                ident,
                format!(
                    "{LABEL} cannot place a `#[map(skip)]` field: it is never stored, so it \
                     has no key for a form to name"
                ),
            ));
        }
        if !skip {
            out.push(FieldDecl {
                key,
                ty: field.ty.clone(),
                hints,
            });
        }
    }
    Ok(out)
}

fn read_hints(attr: &Attribute, hints: &mut FieldHints) -> syn::Result<()> {
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("widget") {
            hints.widget = Some(meta.value()?.parse::<LitStr>()?.value());
        } else if meta.path.is_ident("placeholder") {
            hints.placeholder = Some(meta.value()?.parse::<LitStr>()?.value());
        } else if meta.path.is_ident("nested") {
            hints.nested = true;
        } else if meta.path.is_ident("visible_when") {
            let mut field = None;
            let mut equals = None;
            meta.parse_nested_meta(|inner| {
                if inner.path.is_ident("field") {
                    field = Some(inner.value()?.parse::<LitStr>()?.value());
                } else if inner.path.is_ident("equals") {
                    equals = Some(inner.value()?.parse::<syn::Expr>()?);
                } else {
                    return Err(inner.error(format!(
                        "unrecognised `visible_when` option. {LABEL} knows `field = \"..\"` \
                         and `equals = <value>`"
                    )));
                }
                Ok(())
            })?;
            match (field, equals) {
                (Some(field), Some(equals)) => hints.visible_when = Some((field, equals)),
                _ => {
                    return Err(meta.error(
                        "`visible_when` names both `field = \"..\"` and `equals = <value>`",
                    ));
                }
            }
        } else if meta.path.is_ident("section") {
            return Err(meta.error(format!(
                "a field's section is the schema's: write `#[schema(section = \"..\")]`. \
                 {LABEL} declares what a section is CALLED, on the type: \
                 `#[form(section(id = \"..\", label = \"..\"))]`"
            )));
        } else {
            return Err(meta.error(format!(
                "unrecognised `#[form(..)]` option. {LABEL} knows `widget`, `placeholder`, \
                 `visible_when(field = \"..\", equals = <value>)` and `nested` on a field"
            )));
        }
        Ok(())
    })
}

fn emit(name: &syn::Ident, sections: &[SectionDecl], fields: &[FieldDecl]) -> TokenStream {
    let sections = sections.iter().map(|s| {
        let id = &s.id;
        let label = s.label.iter().map(|l| quote! { .label(#l) });
        let help = s.help.iter().map(|h| quote! { .help(#h) });
        quote! {
            __form = __form.section(
                ::guatiao_form::Section::new_in(__alloc, #id) #(#label)* #(#help)*
            );
        }
    });
    let hints = fields.iter().filter(|f| !f.hints.is_empty()).map(|f| {
        let key = &f.key;
        let ty = &f.ty;
        let own = {
            let widget = f.hints.widget.iter().map(|w| quote! { .widget(#w) });
            let placeholder = f.hints.placeholder.iter().map(|p| quote! { .placeholder(#p) });
            let visible = f
                .hints
                .visible_when
                .iter()
                .map(|(field, equals)| quote! { .visible_when(#field, #equals) });
            if f.hints.widget.is_some()
                || f.hints.placeholder.is_some()
                || f.hints.visible_when.is_some()
            {
                quote! {
                    __form = __form.field(
                        &::std::format!("{}{}", __prefix, #key),
                        ::guatiao_form::Hints::new_in(__alloc) #(#widget)* #(#placeholder)* #(#visible)*,
                    );
                }
            } else {
                quote! {}
            }
        };
        let nested = if f.hints.nested {
            quote! {
                __form = <#ty as ::guatiao_form::Screen>::hints(
                    &::std::format!("{}{}.", __prefix, #key),
                    __form,
                );
            }
        } else {
            quote! {}
        };
        quote! { #own #nested }
    });

    quote! {
        #[automatically_derived]
        impl ::guatiao_form::Screen for #name {
            fn form(
                __alloc: ::guatiao::Alloc,
            ) -> ::core::result::Result<::guatiao::Value, ::guatiao::ValueError> {
                let mut __form = ::guatiao_form::Form::new_in(__alloc);
                #(#sections)*
                __form = <#name as ::guatiao_form::Screen>::hints("", __form);
                __form.finish()
            }

            fn hints(__prefix: &str, mut __form: ::guatiao_form::Form) -> ::guatiao_form::Form {
                // Bound first: `field` takes the form by value.
                let __alloc = __form.alloc();
                #(#hints)*
                __form
            }
        }
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
    fn sections_and_hints_become_builder_calls() {
        let out = expand_str(
            "#[form(section(id = \"net\", label = \"Network\", help = \"Where\"), section(id = \"auth\"))]
             struct Conn {
                 #[schema(section = \"net\")] host: String,
                 #[form(widget = \"password\")] #[map(rename = \"api-token\")] token: String,
                 verify: bool,
                 #[form(placeholder = \"/etc/ca.pem\", visible_when(field = \"verify\", equals = true))] ca: Option<String>,
                 #[form(nested)] auth: Auth,
                 #[map(skip)] cache: Vec<u8>,
                 plain: i64,
             }",
        )
        .unwrap();
        let file: syn::File = syn::parse_str(&out).expect("parses");
        assert_eq!(file.items.len(), 1);
        assert!(out.contains(
            "Section :: new_in (__alloc , \"net\") . label (\"Network\") . help (\"Where\")"
        ));
        assert!(
            out.contains("Section :: new_in (__alloc , \"auth\")) ;"),
            "{out}"
        );
        assert!(out.contains("format ! (\"{}{}\" , __prefix , \"api-token\") , :: guatiao_form :: Hints :: new_in (__alloc) . widget (\"password\")"));
        assert!(out.contains(". placeholder (\"/etc/ca.pem\") . visible_when (\"verify\" , true)"));
        assert!(out.contains("< Auth as :: guatiao_form :: Screen > :: hints (& :: std :: format ! (\"{}{}.\" , __prefix , \"auth\") , __form ,)"));
        assert!(
            !out.contains("\"host\"") && !out.contains("\"plain\"") && !out.contains("\"cache\""),
            "a field without hints is not named: {out}"
        );
    }

    #[test]
    fn an_enum_places_its_arm_fields() {
        let out = expand_str(
            "#[map(tag = \"auth\")] enum Auth { Ambient, UserPass { username: String, #[form(widget = \"password\")] password: String } }",
        )
        .unwrap();
        assert!(out.contains("format ! (\"{}{}\" , __prefix , \"password\")"));
        assert!(!out.contains("\"username\""));
    }

    #[test]
    fn form_rejects_by_name() {
        let cases: &[(&str, &str)] = &[
            ("struct S<T>(T);", "refuses a generic type"),
            ("struct S(String);", "needs named fields"),
            (
                "enum E { A, B }",
                "nothing to place on an enum of unit variants",
            ),
            (
                "enum E { A(String) }",
                "needs a variant's fields to have names",
            ),
            (
                "enum E { #[form(widget = \"x\")] A { b: i64 } }",
                "no `#[form(..)]` on a variant",
            ),
            (
                "union U { a: i64 }",
                "supports structs with named fields and enums",
            ),
            (
                "#[form(widget = \"x\")] struct S { a: i64 }",
                "unrecognised `#[form(..)]` option on a type",
            ),
            (
                "#[form(section(label = \"x\"))] struct S { a: i64 }",
                "a section needs `id",
            ),
            (
                "#[form(section(id = \"x\", colour = \"y\"))] struct S { a: i64 }",
                "unrecognised section option",
            ),
            (
                "struct S { #[form(section = \"x\")] a: i64 }",
                "a field's section is the schema's",
            ),
            (
                "struct S { #[form(colour = \"x\")] a: i64 }",
                "unrecognised `#[form(..)]` option",
            ),
            (
                "struct S { #[form(visible_when(field = \"b\"))] a: i64 }",
                "names both `field",
            ),
            (
                "struct S { #[form(visible_when(field = \"b\", is = 1))] a: i64 }",
                "unrecognised `visible_when` option",
            ),
            (
                "struct S { #[map(skip)] #[form(widget = \"x\")] a: i64 }",
                "cannot place a `#[map(skip)]` field",
            ),
        ];
        for (item, expected) in cases {
            let err = expand_str(item).expect_err(item);
            assert!(
                err.contains(expected),
                "{item}\n  got: {err}\n  want: {expected}"
            );
        }
    }
}
