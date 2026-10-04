// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where generated code finds `guatiao`.
//!
//! Every path an expansion emits is rooted at `::guatiao`, which a consumer
//! must then name as a direct dependency. `crate = <path>` on a derive or
//! on `#[kind]` roots them at a re-export instead, so a crate that defines
//! kinds can be the one thing its providers depend on.

#![forbid(unsafe_code)]

use proc_macro2::{Group, Spacing, TokenStream, TokenTree};
use quote::quote;

/// `tokens` with every path rooted at `::guatiao` rooted at `root`
/// instead; unchanged when `root` is `None`.
///
/// Done over the finished expansion rather than at each `quote!`, so an
/// emitter writes `::guatiao::` and cannot forget the override.
pub(crate) fn reroot(tokens: TokenStream, root: Option<&syn::Path>) -> TokenStream {
    match root {
        Some(root) => rewrite(tokens, &quote!(#root)),
        None => tokens,
    }
}

fn rewrite(tokens: TokenStream, root: &TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let mut out: Vec<TokenTree> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if is_root(&tokens, i) {
            out.extend(root.clone());
            i += 3;
            continue;
        }
        match &tokens[i] {
            TokenTree::Group(group) => {
                let mut inner = Group::new(group.delimiter(), rewrite(group.stream(), root));
                inner.set_span(group.span());
                out.push(inner.into());
            }
            other => out.push(other.clone()),
        }
        i += 1;
    }
    out.into_iter().collect()
}

/// Whether `::guatiao` starts at `i` as the ROOT of a path: `::`, then the
/// ident, with nothing before the `::` that it could be separating from.
fn is_root(tokens: &[TokenTree], i: usize) -> bool {
    let joint = matches!(
        tokens.get(i),
        Some(TokenTree::Punct(p)) if p.as_char() == ':' && p.spacing() == Spacing::Joint
    );
    if !joint || !matches!(tokens.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == ':') {
        return false;
    }
    if !matches!(tokens.get(i + 2), Some(TokenTree::Ident(ident)) if ident == "guatiao") {
        return false;
    }
    match i.checked_sub(1).and_then(|before| tokens.get(before)) {
        // `x::guatiao`, `self::guatiao`: a segment of somebody's path. A
        // keyword that cannot end a path (`as`, `impl`, `dyn`, `for`) is
        // followed by a root.
        Some(TokenTree::Ident(before)) => {
            let text = before.to_string();
            !matches!(text.as_str(), "self" | "super" | "crate" | "Self")
                && syn::parse_str::<syn::Ident>(&text).is_err()
        }
        // After punctuation -- `->`, `<`, `&`, `impl<T>` -- a path starts.
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rerooted(tokens: TokenStream) -> String {
        let root: syn::Path = syn::parse_quote!(::kinds::guatiao);
        reroot(tokens, Some(&root)).to_string()
    }

    #[test]
    fn a_root_is_replaced_wherever_a_path_can_start() {
        assert_eq!(
            rerooted(quote! { ::guatiao::Value }),
            quote! { ::kinds::guatiao::Value }.to_string()
        );
        // After a keyword, inside a qualified path, and inside a group.
        assert_eq!(
            rerooted(quote! {
                impl ::guatiao::ToValue for S {
                    fn f() -> ::guatiao::Value { <u8 as ::guatiao::ToValue>::to_value(&1) }
                }
            }),
            quote! {
                impl ::kinds::guatiao::ToValue for S {
                    fn f() -> ::kinds::guatiao::Value { <u8 as ::kinds::guatiao::ToValue>::to_value(&1) }
                }
            }
            .to_string()
        );
    }

    #[test]
    fn a_segment_of_another_path_is_left_alone() {
        for kept in [
            quote! { kinds::guatiao::Value },
            quote! { crate::guatiao::Value },
            quote! { self::guatiao::Value },
            quote! { ::guatiao_intake::Screen },
        ] {
            assert_eq!(rerooted(kept.clone()), kept.to_string());
        }
    }

    #[test]
    fn no_override_changes_nothing() {
        let tokens = quote! { ::guatiao::Value };
        assert_eq!(reroot(tokens.clone(), None).to_string(), tokens.to_string());
    }
}
