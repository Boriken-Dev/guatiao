// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The helper program: which one to use, what C library versions it needs,
//! and building one while the host runs.

#![forbid(unsafe_code)]

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::elf::Elf;
use crate::error::{Error, ImageError};

include!("../helper/flags.rs");

#[cfg(guatiao_foreign_helper)]
static BUILT: Option<&[u8]> = Some(include_bytes!(concat!(env!("OUT_DIR"), "/helper")));
#[cfg(not(guatiao_foreign_helper))]
static BUILT: Option<&[u8]> = None;

/// The name the helper runs under (its `argv[0]`) when it has no path.
pub(crate) const BUILT_NAME: &str = "guatiao-load-foreign-helper";

/// Which helper program `ld.so` starts: a glibc program, position-independent
/// and naming its interpreter, built from [`Helper::SOURCE`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum Helper {
    /// The one built into this crate at build time (none when it was
    /// built with `GUATIAO_FOREIGN_HELPER_CC=none`).
    #[default]
    Built,
    /// A file, read when the loader starts.
    Path(PathBuf),
    /// Bytes the host holds.
    Bytes(Arc<[u8]>),
}

/// One version a program needs from a library it links: `GLIBC_2.17` of
/// `libc.so.6`, for example.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionNeed {
    /// The library's soname.
    pub file: String,
    /// The version.
    pub version: String,
}

impl Helper {
    /// The helper's C source, the same text the crate's own helper was
    /// built from.
    pub const SOURCE: &'static str = include_str!("../helper/helper.c");

    /// The helper built into this crate, if it was built with one.
    pub fn built() -> Option<&'static [u8]> {
        BUILT
    }

    /// The program's bytes, and the path they came from.
    pub(crate) fn bytes(&self) -> Result<(Cow<'_, [u8]>, Option<&Path>), Error> {
        match self {
            Helper::Built => BUILT
                .map(|b| (Cow::Borrowed(b), None))
                .ok_or(Error::NoHelper),
            Helper::Path(path) => std::fs::read(path)
                .map(|b| (Cow::Owned(b), Some(path.as_path())))
                .map_err(|e| Error::Helper {
                    path: Some(path.clone()),
                    reason: ImageError::Io(e),
                }),
            Helper::Bytes(bytes) => Ok((Cow::Borrowed(bytes), None)),
        }
    }

    /// The C library versions the helper needs, read from its
    /// `.gnu.version_r` section. The newest of them is the oldest C library
    /// it runs on.
    pub fn needs(&self) -> Result<Vec<VersionNeed>, Error> {
        let (bytes, path) = self.bytes()?;
        let wrap = |reason| Error::Helper {
            path: path.map(Path::to_path_buf),
            reason,
        };
        Elf::parse(&bytes)
            .and_then(|elf| elf.version_needs())
            .map_err(wrap)
    }

    /// Builds the helper with `compiler` (a C compiler targeting glibc on
    /// this architecture) into `out`, writing the source beside it as
    /// `out` with `.c` appended, and answers [`Helper::Path`] for it.
    pub fn build(compiler: &Path, out: &Path) -> Result<Helper, Error> {
        let mut source = out.as_os_str().to_owned();
        source.push(".c");
        let source = PathBuf::from(source);
        std::fs::write(&source, Self::SOURCE).map_err(|e| Error::Build {
            status: alloc::format!("writing {}", source.display()),
            stderr: e.to_string(),
        })?;
        let output = Command::new(compiler)
            .args(HELPER_FLAGS)
            .arg("-o")
            .arg(out)
            .arg(&source)
            .args(HELPER_LIBS)
            .output();
        let _ = std::fs::remove_file(&source);
        match output {
            Ok(o) if o.status.success() => Ok(Helper::Path(out.to_path_buf())),
            Ok(o) => Err(Error::Build {
                status: o.status.to_string(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            }),
            Err(e) => Err(Error::Build {
                status: alloc::format!("{} could not run", compiler.display()),
                stderr: e.to_string(),
            }),
        }
    }
}
