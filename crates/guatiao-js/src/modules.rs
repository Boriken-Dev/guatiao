// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Which modules a library can import: the three it is given
//! (`guatiao`, `guatiao/kinds`, `guatiao/provide`) and the files under
//! its own paths, by relative specifier.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use core::cell::RefCell;
use std::path::{Component, Path, PathBuf};

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{Ctx, Error, Module, Result};

/// The modules a library is given, by the name it imports them as.
const BUILTIN: [(&str, &str); 3] = [
    ("guatiao", include_str!("prelude/guatiao.js")),
    ("guatiao/kinds", include_str!("prelude/kinds.js")),
    ("guatiao/provide", include_str!("prelude/provide.js")),
];

/// The paths each open context may import files from, by context.
#[derive(Clone, Default)]
pub(crate) struct Roots(Rc<RefCell<BTreeMap<usize, Vec<PathBuf>>>>);

impl Roots {
    fn key(ctx: &Ctx<'_>) -> usize {
        ctx.as_raw().as_ptr() as usize
    }

    /// Lets `ctx` import files under `paths`.
    pub(crate) fn allow(&self, ctx: &Ctx<'_>, paths: Vec<PathBuf>) {
        self.0.borrow_mut().insert(Self::key(ctx), paths);
    }

    /// Takes the permission back.
    pub(crate) fn forget(&self, ctx: &Ctx<'_>) {
        self.0.borrow_mut().remove(&Self::key(ctx));
    }

    fn allows(&self, ctx: &Ctx<'_>, file: &Path) -> bool {
        self.0
            .borrow()
            .get(&Self::key(ctx))
            .is_some_and(|paths| paths.iter().any(|p| file.starts_with(p)))
    }
}

/// `path` with `.` and `..` resolved by name alone: symbolic links are not
/// followed.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolves what a module imports, to the file's absolute path or a
/// built-in's name.
pub(crate) struct Resolve {
    pub(crate) roots: Roots,
}

impl Resolver for Resolve {
    fn resolve<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<String> {
        if BUILTIN.iter().any(|(n, _)| *n == name) {
            return Ok(name.to_string());
        }
        let relative = name.starts_with("./") || name.starts_with("../");
        if !relative && !Path::new(name).is_absolute() {
            return Err(Error::new_resolving_message(
                base,
                name,
                "a library imports guatiao, guatiao/kinds, guatiao/provide and files by relative path",
            ));
        }
        let joined = if relative {
            Path::new(base)
                .parent()
                .map_or_else(|| PathBuf::from(name), |dir| dir.join(name))
        } else {
            PathBuf::from(name)
        };
        let file = normalize(&joined);
        if !self.roots.allows(ctx, &file) {
            return Err(Error::new_resolving_message(
                base,
                name,
                "outside the paths of the library",
            ));
        }
        Ok(file.display().to_string())
    }
}

/// Loads a built-in's source or a module file's.
pub(crate) struct Load;

impl Loader for Load {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js, Declared>> {
        if let Some((_, source)) = BUILTIN.iter().find(|(n, _)| *n == name) {
            return Module::declare(ctx.clone(), name, *source);
        }
        if attributes.is_some_and(|a| a.keys().next().is_some()) {
            return Err(Error::new_loading_message(
                name,
                "import attributes (JSON modules) are not served",
            ));
        }
        let source =
            std::fs::read(name).map_err(|e| Error::new_loading_message(name, e.to_string()))?;
        Module::declare(ctx.clone(), name, source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_are_resolved_by_name() {
        assert_eq!(
            normalize(Path::new("/a/b/../c/./d.js")),
            PathBuf::from("/a/c/d.js")
        );
        assert_eq!(normalize(Path::new("/a/../../b")), PathBuf::from("/b"));
    }
}
