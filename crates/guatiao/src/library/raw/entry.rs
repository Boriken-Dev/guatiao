// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a library writes: the entry point's symbol and type, the
//! macros that emit it, and the declaration a scanner reads as data.

use super::*;

/// The symbol a library exports, NUL-terminated for the loader.
pub const ENTRY_SYMBOL: &[u8] = b"guatiao_library_entry\0";

/// The data symbol a library declares itself under, NUL-terminated for a
/// loader that looks it up by name.
///
/// Its bytes are `key=value` strings separated by NUL and ended by an
/// empty string, so the whole declaration ends in two NULs. Every kind
/// a provider serves is `kind=<name>`; a library adds pairs of its own.
/// Written by [`declares!`](crate::declares) or
/// [`providers!`](crate::providers); read by a scanner **as data**,
/// before the file is mapped, so a host can keep a library out of a scan
/// by what it declares rather than by its filename.
pub const DECLARES_SYMBOL: &[u8] = b"guatiao_declares\0";

/// How many bytes a declaration takes: `kind=<name>` for every kind of
/// every provider, then each extra pair, each NUL-terminated, then the
/// empty string that ends it. Evaluated at compile time by the macros.
#[doc(hidden)]
pub const fn declaration_len(kinds: &[&[&str]], extra: &[&str]) -> usize {
    let mut n = 1;
    let mut i = 0;
    while i < kinds.len() {
        let mut j = 0;
        while j < kinds[i].len() {
            n += "kind=".len() + kinds[i][j].len() + 1;
            j += 1;
        }
        i += 1;
    }
    let mut i = 0;
    while i < extra.len() {
        n += extra[i].len() + 1;
        i += 1;
    }
    n
}

/// The bytes of a declaration, sized by [`declaration_len`]. A pair that
/// is not `KEY=VALUE`, or holds a NUL, is refused at compile time.
#[doc(hidden)]
pub const fn declaration<const N: usize>(kinds: &[&[&str]], extra: &[&str]) -> [u8; N] {
    let mut out = [0u8; N];
    let mut at = 0;
    let mut i = 0;
    while i < kinds.len() {
        let mut j = 0;
        while j < kinds[i].len() {
            at = put(&mut out, at, b"kind=");
            at = put(&mut out, at, kinds[i][j].as_bytes());
            at += 1;
            j += 1;
        }
        i += 1;
    }
    let mut i = 0;
    while i < extra.len() {
        let pair = extra[i].as_bytes();
        let mut has_eq = false;
        let mut k = 0;
        while k < pair.len() {
            if pair[k] == b'=' && k > 0 {
                has_eq = true;
            }
            if pair[k] == 0 {
                panic!("a declaration cannot hold a NUL");
            }
            k += 1;
        }
        if !has_eq {
            panic!("a declaration is `KEY=VALUE`");
        }
        at = put(&mut out, at, pair);
        at += 1;
        i += 1;
    }
    // `at + 1 == N` by construction; the ending empty string is the zero
    // already there.
    out
}

const fn put(out: &mut [u8], at: usize, bytes: &[u8]) -> usize {
    let mut k = 0;
    while k < bytes.len() {
        out[at + k] = bytes[k];
        k += 1;
    }
    at + bytes.len()
}

/// The entry point's signature.
///
/// Returning null means **"nothing for this host"**, which is an answer
/// rather than a failure: a library that supports one host and is loaded
/// by another says so this way, and the loader reports it as skipped.
pub type EntryFn = unsafe extern "C" fn(*const HostInfo<'static>) -> *const LibraryInfo<'static>;

/// Writes a library's entry point.
///
/// Give it a function taking a [`Host`] and answering
/// `Option<&'static LibraryInfo<'static>>`; it emits the `extern "C"` symbol a
/// loader looks for, catches a panic rather than letting one cross the
/// boundary, and answers null for "nothing for this host". The `Host` is
/// `Copy + Send + Sync + 'static`, so a library keeps it in a `OnceLock`
/// of its own and asks the host for a provider from any later call.
///
/// ```ignore
/// fn describe(host: guatiao::library::Host) -> Option<&'static guatiao::library::LibraryInfo<'static>> {
///     // build a descriptor, keep it and `host` in a `OnceLock` of your own
/// }
/// guatiao::guatiao_library!(describe);
/// ```
///
/// # Why a macro rather than a documented signature
///
/// So a library author never writes `unsafe` and never writes
/// `no_mangle` — a misspelled symbol name is a library that loads and
/// offers nothing, which is the failure with the least evidence attached
/// to it. The expansion also holds the unwind discipline the rest of this
/// crate's boundaries use: a panic becomes null, and the payload is
/// forgotten rather than dropped, because dropping it can panic again and
/// a second panic in an `extern "C"` body is an abort.
#[macro_export]
macro_rules! guatiao_library {
    ($describe:path) => {
        /// The symbol a `guatiao` host loads this library by.
        ///
        /// # Safety
        ///
        /// Called by a loader with either null or a pointer to a
        /// well-formed host descriptor.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn guatiao_library_entry(
            host: *const $crate::library::HostInfo<'static>,
        ) -> *const $crate::library::LibraryInfo<'static> {
            // SAFETY: the loader's side of the contract is that `host` is
            // null or points at a host descriptor declaring its own size
            // that stays valid for the life of the process.
            let host = unsafe { $crate::library::Host::from_raw(host) };
            let Some(host) = host else {
                return ::core::ptr::null();
            };
            $crate::library::answer(|| $describe(host))
        }
    };
}

/// Writes what a library declares, for a scanner to read before mapping
/// it.
///
/// Each argument is a `KEY=VALUE` literal. A host's scan rules match
/// against these (`guatiao_registry_scan_dir_rules`, `ScanRules`), so a
/// library that must never be loaded by a scan says so here and the host
/// writes one rule rather than a filename denylist:
///
/// ```ignore
/// guatiao::declares!("kind=greeter", "kind=writer", "VIEWER=1");
/// ```
///
/// A library built with [`providers!`](crate::providers) declares its
/// kinds by itself and takes extra pairs through `declares = [..]`; this
/// is for a hand-written one. A library declaring nothing is loaded by a
/// plain scan as before, and fails every positive rule.
#[macro_export]
macro_rules! declares {
    ($($pair:expr),* $(,)?) => {
        $crate::__guatiao_declares!(kinds = [], pairs = [$($pair),*]);
    };
}

/// The `static` behind [`declares!`] and [`providers!`].
#[doc(hidden)]
#[macro_export]
macro_rules! __guatiao_declares {
    (kinds = [$($kinds:expr),*], pairs = [$($pair:expr),*]) => {
        const __GUATIAO_DECLARED_KINDS: &[&[&str]] = &[$($kinds),*];
        const __GUATIAO_DECLARED_PAIRS: &[&str] = &[$($pair),*];
        const __GUATIAO_DECLARES_LEN: usize = $crate::library::declaration_len(
            __GUATIAO_DECLARED_KINDS,
            __GUATIAO_DECLARED_PAIRS,
        );
        /// What this library declares, read by a scanner as data.
        #[allow(non_upper_case_globals)]
        #[unsafe(no_mangle)]
        pub static guatiao_declares: [u8; __GUATIAO_DECLARES_LEN] =
            $crate::library::declaration(__GUATIAO_DECLARED_KINDS, __GUATIAO_DECLARED_PAIRS);
    };
}

/// Writes a whole library from its provider types.
///
/// `providers!(A, B)` names the types that `#[derive(Provider)]` made
/// providers of; the library's id and version default to the crate's own
/// `CARGO_PKG_NAME` and `CARGO_PKG_VERSION`. The long form overrides them:
///
/// ```ignore
/// guatiao::providers!(Hello);
/// guatiao::providers!(id = "acme_net", version = "1.2.0", providers = [Hello, Counter]);
/// ```
///
/// Each provider's descriptor is built once, on the first entry call,
/// and kept for the process. A provider whose configuration schema cannot
/// be built makes the library decline the host.
///
/// The library also **declares** every kind its providers serve, as
/// data a scanner reads before mapping it; `providers!(A, B; declares =
/// ["VIEWER=1"])` (or `declares = [..]` after `providers = [..]` in
/// the long form) adds pairs of the library's own. See
/// [`declares!`](crate::declares).
///
/// The library **refuses to be unloaded while anything it handed out is
/// alive**: an instance built from a configuration, or an object one of
/// its kinds returned. The derives count them, and the `unload` slot this
/// writes answers `GUATIAO_ERR_BUSY` until the count is zero. The
/// long form also takes `unload = <fn>`, an
/// `unsafe extern "C" fn() -> Status` asked after that, for what only the
/// library knows. See [`LibraryInfo::unload`].
#[macro_export]
macro_rules! providers {
    ($($provider:ty),+ $(,)?) => {
        $crate::providers!(
            id = env!("CARGO_PKG_NAME"),
            version = env!("CARGO_PKG_VERSION"),
            providers = [$($provider),+]
        );
    };
    ($($provider:ty),+ ; declares = [$($pair:expr),* $(,)?]) => {
        $crate::providers!(
            id = env!("CARGO_PKG_NAME"),
            version = env!("CARGO_PKG_VERSION"),
            providers = [$($provider),+],
            declares = [$($pair),*]
        );
    };
    (
        id = $id:expr,
        version = $version:expr,
        providers = [$($provider:ty),+ $(,)?]
        $(, declares = [$($pair:expr),* $(,)?])?,
        unload = $unload:expr $(,)?
    ) => {
        $crate::__guatiao_declares!(
            kinds = [$(<$provider as $crate::library::kind::ProviderDecl>::KINDS),+],
            pairs = [$($($pair),*)?]
        );
        $crate::__guatiao_describe!(
            #[doc(hidden)]
            fn __guatiao_describe;
            id = $id, version = $version, providers = [$($provider),+],
            unload = ::core::option::Option::Some(__guatiao_unload)
        );
        $crate::__guatiao_unload!(
            providers = [$($provider),+],
            // SAFETY: the library's own slot, called as the host would.
            then = unsafe { ($unload)() }
        );
        $crate::guatiao_library!(__guatiao_describe);
    };
    (
        id = $id:expr,
        version = $version:expr,
        providers = [$($provider:ty),+ $(,)?]
        $(, declares = [$($pair:expr),* $(,)?])? $(,)?
    ) => {
        $crate::__guatiao_declares!(
            kinds = [$(<$provider as $crate::library::kind::ProviderDecl>::KINDS),+],
            pairs = [$($($pair),*)?]
        );
        $crate::__guatiao_describe!(
            #[doc(hidden)]
            fn __guatiao_describe;
            id = $id, version = $version, providers = [$($provider),+],
            unload = ::core::option::Option::Some(__guatiao_unload)
        );
        $crate::__guatiao_unload!(
            providers = [$($provider),+],
            then = $crate::Status::GUATIAO_OK
        );
        $crate::guatiao_library!(__guatiao_describe);
    };
}

/// The `unload` slot behind [`providers!`](crate::providers): refuses
/// while any provider reports something alive, then answers `then`.
#[doc(hidden)]
#[macro_export]
macro_rules! __guatiao_unload {
    (providers = [$($provider:ty),+], then = $then:expr) => {
        #[doc(hidden)]
        unsafe extern "C" fn __guatiao_unload() -> $crate::Status {
            $crate::library::kind::catch(|| {
                let mut seen = ::std::vec::Vec::new();
                let live = 0usize
                    $(+ <$provider as $crate::library::kind::ProviderDecl>::live(&mut seen))+;
                if live > 0 {
                    return $crate::Status::GUATIAO_ERR_BUSY;
                }
                $then
            })
        }
    };
}

/// Writes a library the host **links** rather than loads.
///
/// The same as [`providers!`](crate::providers) without the two exported symbols: no entry
/// point, no declaration, so the artifact this ends up in — a host's own
/// binary, or an engine that carries one provider compiled in — does not
/// look like a plugin to a scan of its directory. What it writes instead
/// is one function, `library`, which the host hands to
/// [`Registry::register_local`](crate::library::Registry::register_local):
///
/// ```ignore
/// guatiao::local_providers!(Remote);                       // in the host's crate
/// registry.register_local("engine", crate::library)?;      // beside what it loads
/// ```
///
/// The providers are built once, on the first call, and kept for the
/// process, as a loaded library's are.
#[macro_export]
macro_rules! local_providers {
    ($($provider:ty),+ $(,)?) => {
        $crate::local_providers!(
            id = env!("CARGO_PKG_NAME"),
            version = env!("CARGO_PKG_VERSION"),
            providers = [$($provider),+]
        );
    };
    (
        id = $id:expr,
        version = $version:expr,
        providers = [$($provider:ty),+ $(,)?] $(,)?
    ) => {
        $crate::__guatiao_describe!(
            /// What this crate offers as a library the host links: hand
            /// it to `Registry::register_local`.
            pub fn library;
            id = $id, version = $version, providers = [$($provider),+],
            unload = ::core::option::Option::None
        );
    };
}

/// The describe function behind [`providers!`](crate::providers) and
/// [`local_providers!`](crate::local_providers).
#[doc(hidden)]
#[macro_export]
macro_rules! __guatiao_describe {
    (
        $(#[$attr:meta])* $vis:vis fn $name:ident;
        id = $id:expr, version = $version:expr, providers = [$($provider:ty),+],
        unload = $unload:expr
    ) => {
        $(#[$attr])*
        $vis fn $name(
            host: $crate::library::Host,
        ) -> ::core::option::Option<&'static $crate::library::LibraryInfo<'static>> {
            static REGISTERED: ::std::sync::OnceLock<::core::option::Option<$crate::library::kind::LibraryParts>> = ::std::sync::OnceLock::new();
            REGISTERED
                .get_or_init(|| {
                    // The host's arena when it offers one, so what this
                    // library builds for the host outlives this mapping.
                    let alloc = host.alloc().unwrap_or_else($crate::Alloc::rust);
                    let mut providers = ::std::vec::Vec::new();
                    $(
                        providers.push(
                            <$provider as $crate::library::kind::ProviderDecl>::provider(host, alloc).ok()?,
                        );
                    )+
                    ::core::option::Option::Some(
                        $crate::library::kind::LibraryParts::new($id, $version, providers)
                            .unloading($unload),
                    )
                })
                .as_ref()
                .map($crate::library::kind::LibraryParts::info)
        }
    };
}

/// Runs a library's `describe` and converts the answer to what the entry
/// point returns.
///
/// A panic becomes null, which a loader reports as "declined" rather than
/// as a crash.
pub fn answer(
    describe: impl FnOnce() -> Option<&'static LibraryInfo<'static>>,
) -> *const LibraryInfo<'static> {
    match catch_unwind(AssertUnwindSafe(describe)) {
        Ok(Some(desc)) => desc as *const LibraryInfo,
        Ok(None) => std::ptr::null(),
        Err(payload) => {
            // Dropping a panic payload can panic, and a second panic
            // inside an `extern "C"` body is an abort.
            std::mem::forget(payload);
            std::ptr::null()
        }
    }
}
