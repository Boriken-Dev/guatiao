// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Which type a kind signature names, read from how it is spelled.
//!
//! A proc macro sees tokens, not types. The few types a kind's glue treats
//! specially are recognised here under the spellings that can only mean
//! them, and every type is rewritten to one canonical spelling for the
//! table hash, so `std::vec::Vec<T>` and `Vec<T>` hash alike. A spelling
//! not listed here — an alias, a `use .. as`, a path through a module of
//! the caller's — is taken as written: the macro cannot see what it
//! names.

use syn::{GenericArgument, Path, PathArguments, Type, TypeParamBound};

/// `guatiao`'s types, recognised bare or under the crate's root.
const GUATIAO_NAMES: &[&str] = &[
    "Value",
    "Map",
    "List",
    "Text",
    "ProviderError",
    "Object",
    "Status",
];

/// The standard library's, recognised bare or under `std`, `core` or
/// `alloc`.
const STD_NAMES: &[&str] = &["Vec", "String", "Option", "Result", "Box"];

/// The crate roots a spelling may start from.
#[derive(Clone, Copy)]
pub(crate) struct Roots<'a> {
    /// `crate = <path>` on the attribute, beside `guatiao` itself.
    krate: Option<&'a Path>,
}

impl<'a> Roots<'a> {
    pub(crate) fn new(krate: Option<&'a Path>) -> Roots<'a> {
        Roots { krate }
    }

    /// The name of the `guatiao` type `path` spells: bare, under
    /// `guatiao` or `::guatiao`, or under the `crate =` root. `None` for
    /// anything else, a path ending in one of those names included.
    pub(crate) fn guatiao(&self, path: &Path) -> Option<&'static str> {
        let (prefix, name) = split(path)?;
        let name = *GUATIAO_NAMES.iter().find(|n| **n == name)?;
        if prefix.is_empty() && path.leading_colon.is_none() {
            return Some(name);
        }
        if prefix.first().is_some_and(|s| s == "guatiao") {
            return Some(name);
        }
        let root: Vec<String> = self
            .krate?
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        prefix.starts_with(&root).then_some(name)
    }

    /// The name of the standard type `path` spells: bare, or under
    /// `std`, `core` or `alloc`.
    pub(crate) fn std(&self, path: &Path) -> Option<&'static str> {
        let (prefix, name) = split(path)?;
        let name = *STD_NAMES.iter().find(|n| **n == name)?;
        let bare = prefix.is_empty() && path.leading_colon.is_none();
        let under = prefix
            .first()
            .is_some_and(|s| matches!(s.as_str(), "std" | "core" | "alloc"));
        (bare || under).then_some(name)
    }

    /// The name of guatiao's or the standard library's type that `path`
    /// ends in while spelled under a root this macro does not know
    /// (`foreign::Value`). `None` when it is recognised, or ends in no such
    /// name.
    pub(crate) fn unrecognised(&self, path: &Path) -> Option<&'static str> {
        if self.guatiao(path).is_some() || self.std(path).is_some() {
            return None;
        }
        let (_, name) = split(path)?;
        GUATIAO_NAMES
            .iter()
            .chain(STD_NAMES)
            .find(|n| **n == name)
            .copied()
    }

    /// The refusal for a type [`Roots::unrecognised`] named, saying which
    /// spellings are read.
    pub(crate) fn refusal(&self, spanned: &impl quote::ToTokens, name: &str) -> syn::Error {
        let accepted = if STD_NAMES.contains(&name) {
            format!(
                "the standard `{name}` is spelled `{name}` or under `std::`, `core::` or `alloc::`"
            )
        } else {
            format!(
                "guatiao's `{name}` is spelled `{name}`, or under `guatiao::`, `::guatiao::` or \
                 the `crate =` root"
            )
        };
        syn::Error::new_spanned(
            spanned,
            format!(
                "this type ends in `{name}` under a path the kind attribute does not read, so it \
                 cannot tell which `{name}` it is: {accepted}; a type of your own needs another \
                 name in the signature (`use .. as ..`)"
            ),
        )
    }

    /// `ty` with every recognised path bare, no leading `::`, no
    /// lifetime on a reference and no parentheses: what the hash reads.
    pub(crate) fn canonical(&self, ty: &Type) -> Type {
        let mut ty = ty.clone();
        self.rewrite(&mut ty);
        ty
    }

    fn rewrite(&self, ty: &mut Type) {
        match ty {
            Type::Path(p) => {
                if let Some(q) = &mut p.qself {
                    self.rewrite(&mut q.ty);
                }
                self.rewrite_path(&mut p.path, p.qself.is_none());
            }
            Type::Reference(r) => {
                r.lifetime = None;
                self.rewrite(&mut r.elem);
            }
            Type::Slice(s) => self.rewrite(&mut s.elem),
            Type::Array(a) => self.rewrite(&mut a.elem),
            Type::Ptr(p) => self.rewrite(&mut p.elem),
            Type::Tuple(t) => t.elems.iter_mut().for_each(|e| self.rewrite(e)),
            Type::Paren(p) => {
                *ty = (*p.elem).clone();
                self.rewrite(ty);
            }
            Type::Group(g) => {
                *ty = (*g.elem).clone();
                self.rewrite(ty);
            }
            Type::TraitObject(t) => {
                for bound in t.bounds.iter_mut() {
                    if let TypeParamBound::Trait(b) = bound {
                        self.rewrite_path(&mut b.path, false);
                    }
                }
            }
            _ => {}
        }
    }

    /// A recognised type path becomes its last segment alone; any other
    /// keeps its segments and loses only a leading `::`.
    fn rewrite_path(&self, path: &mut Path, may_collapse: bool) {
        if may_collapse
            && (self.guatiao(path).is_some() || self.std(path).is_some())
            && let Some(last) = path.segments.pop()
        {
            path.segments.clear();
            path.segments.push(last.into_value());
        }
        path.leading_colon = None;
        for segment in path.segments.iter_mut() {
            if let PathArguments::AngleBracketed(a) = &mut segment.arguments {
                for arg in a.args.iter_mut() {
                    match arg {
                        GenericArgument::Type(t) => self.rewrite(t),
                        GenericArgument::AssocType(a) => self.rewrite(&mut a.ty),
                        _ => {}
                    }
                }
            }
        }
    }
}

/// The idents before the last segment, and the last one's.
fn split(path: &Path) -> Option<(Vec<String>, String)> {
    let mut idents: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    let last = idents.pop()?;
    Some((idents, last))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn spelled(roots: Roots<'_>, ty: &str) -> String {
        let ty: Type = syn::parse_str(ty).unwrap();
        let canonical = roots.canonical(&ty);
        quote!(#canonical)
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    #[test]
    fn std_and_guatiao_paths_lose_their_prefix() {
        let roots = Roots::new(None);
        for (ty, expected) in [
            ("Vec<Channel>", "Vec<Channel>"),
            ("std::vec::Vec<Channel>", "Vec<Channel>"),
            ("::alloc::vec::Vec<Channel>", "Vec<Channel>"),
            ("::std::string::String", "String"),
            (
                "core::option::Option<&'static ::guatiao::Value>",
                "Option<&Value>",
            ),
            (
                "core::result::Result<Vec<Channel>, guatiao::library::ProviderError>",
                "Result<Vec<Channel>,ProviderError>",
            ),
            (
                "::guatiao::library::Object<dyn ::kinds::Listener>",
                "Object<dynkinds::Listener>",
            ),
            ("(&'a guatiao::Map)", "&Map"),
            ("Box<[u8]>", "Box<[u8]>"),
        ] {
            assert_eq!(spelled(roots, ty), expected, "{ty}");
        }
    }

    #[test]
    fn a_path_of_the_callers_is_taken_as_written() {
        let roots = Roots::new(None);
        for (ty, expected) in [
            ("foreign::Value", "foreign::Value"),
            ("::model::Channel", "model::Channel"),
            ("Vec<RenamedChannel>", "Vec<RenamedChannel>"),
            ("mine::Vec<u8>", "mine::Vec<u8>"),
        ] {
            assert_eq!(spelled(roots, ty), expected, "{ty}");
        }
    }

    #[test]
    fn the_crate_root_counts_as_guatiao() {
        let root: Path = syn::parse_str("::kinds::guatiao").unwrap();
        let roots = Roots::new(Some(&root));
        assert_eq!(spelled(roots, "kinds::guatiao::Value"), "Value");
        assert_eq!(
            spelled(roots, "::kinds::guatiao::library::ProviderError"),
            "ProviderError"
        );
        assert_eq!(spelled(roots, "kinds::Value"), "kinds::Value");
        let path: Path = syn::parse_str("::kinds::guatiao::Map").unwrap();
        assert_eq!(roots.guatiao(&path), Some("Map"));
        let path: Path = syn::parse_str("other::Map").unwrap();
        assert_eq!(roots.guatiao(&path), None);
        let path: Path = syn::parse_str("std::string::String").unwrap();
        assert_eq!(roots.std(&path), Some("String"));
        let path: Path = syn::parse_str("foreign::String").unwrap();
        assert_eq!(roots.std(&path), None);
    }
}
