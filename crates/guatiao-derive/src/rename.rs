// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[map(rename_all = "..")]`: one case rule for every field of a struct
//! or every variant of an enum, spelled and applied as serde's.

use syn::LitStr;

/// A case rule, applied before any `rename` on the field or variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenameRule {
    /// `"lowercase"`.
    Lower,
    /// `"snake_case"`.
    Snake,
    /// `"kebab-case"`.
    Kebab,
    /// `"SCREAMING_SNAKE_CASE"`.
    ScreamingSnake,
}

impl RenameRule {
    /// The rule a `rename_all = ".."` literal names, or why not.
    pub(crate) fn parse(lit: &LitStr) -> syn::Result<RenameRule> {
        Ok(match lit.value().as_str() {
            "lowercase" => RenameRule::Lower,
            "snake_case" => RenameRule::Snake,
            "kebab-case" => RenameRule::Kebab,
            "SCREAMING_SNAKE_CASE" => RenameRule::ScreamingSnake,
            _ => {
                return Err(syn::Error::new_spanned(
                    lit,
                    "`rename_all` takes \"lowercase\", \"snake_case\", \"kebab-case\" or \
                     \"SCREAMING_SNAKE_CASE\", as serde spells them",
                ));
            }
        })
    }

    /// A field's key: the field is `snake_case` already.
    pub(crate) fn field(self, name: &str) -> String {
        match self {
            RenameRule::Lower | RenameRule::Snake => name.to_string(),
            RenameRule::Kebab => name.replace('_', "-"),
            RenameRule::ScreamingSnake => name.to_ascii_uppercase(),
        }
    }

    /// A variant's stored name: the variant is `PascalCase`.
    pub(crate) fn variant(self, name: &str) -> String {
        let snake = || {
            let mut out = String::with_capacity(name.len() + 4);
            for (i, c) in name.char_indices() {
                if c.is_uppercase() && i > 0 {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
            }
            out
        };
        match self {
            RenameRule::Lower => name.to_ascii_lowercase(),
            RenameRule::Snake => snake(),
            RenameRule::Kebab => snake().replace('_', "-"),
            RenameRule::ScreamingSnake => snake().to_ascii_uppercase(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rules_are_serde_s() {
        let rule = |s: &str| RenameRule::parse(&syn::parse_str::<LitStr>(s).unwrap()).unwrap();
        for (lit, variant, field) in [
            ("\"lowercase\"", "highdynamicrange", "view_only"),
            ("\"snake_case\"", "high_dynamic_range", "view_only"),
            ("\"kebab-case\"", "high-dynamic-range", "view-only"),
            (
                "\"SCREAMING_SNAKE_CASE\"",
                "HIGH_DYNAMIC_RANGE",
                "VIEW_ONLY",
            ),
        ] {
            assert_eq!(rule(lit).variant("HighDynamicRange"), variant, "{lit}");
            assert_eq!(rule(lit).field("view_only"), field, "{lit}");
        }
        assert_eq!(rule("\"kebab-case\"").variant("H264"), "h264");
        let refused = RenameRule::parse(&syn::parse_str::<LitStr>("\"camelCase\"").unwrap());
        assert!(
            refused
                .unwrap_err()
                .to_string()
                .contains("as serde spells them")
        );
    }
}
