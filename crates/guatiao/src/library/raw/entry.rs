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

/// The symbol a source file exports, NUL-terminated for the loader.
pub const SOURCE_SYMBOL: &[u8] = b"guatiao_source\0";

/// A source file's entry point: answers its table through `out` for the
/// configuration `config` (a map), or refuses with a status and the
/// source's words in `err`. Called once per `add_source_file`; the table it
/// answers, its slots and its `ctx` stay valid for the life of the process.
pub type SourceFn = unsafe extern "C" fn(
    host: *const HostInfo<'static>,
    config: *const Value,
    out: *mut *const crate::library::desc::SourceInfo,
    err: *mut crate::library::kind::ProviderError,
) -> Status;

// SAFETY: a descriptor is plain data its producer does not write once it
// is answered: moving or sharing one moves addresses, and what they
// address is read under the producer's contract, from whichever thread a
// registry is called on.
unsafe impl Send for LibraryInfo<'_> {}
// SAFETY: as above.
unsafe impl Sync for LibraryInfo<'_> {}

/// A library descriptor a host links rather than loads: what a describe
/// function hands
/// [`Registry::register_local`](crate::library::Registry::register_local).
///
/// Registering it is the host's trust in it, as loading a file is: the
/// registry reads it under the checks every descriptor gets, and its
/// fields are its producer's contract (see [`LibraryInfo`]).
///
/// **Borrowed for the registration only.** The registry copies what the
/// descriptor says — text, values, every table — into the library's
/// lease, and keeps of it only its providers' code and `ctx`, which are
/// the producer's to keep while the library is registered or a handle
/// into it lives. Parts the host built are handed over instead, owned by
/// that lease, and freed with it.
#[derive(Debug)]
pub enum LocalLibrary<'a> {
    /// Parts the registry owns from here on.
    Owned(Box<crate::library::kind::LibraryParts>),
    /// A descriptor read once, at the registration.
    Borrowed(&'a LibraryInfo<'a>),
}

impl LocalLibrary<'_> {
    /// The descriptor.
    pub fn info(&self) -> &LibraryInfo<'_> {
        match self {
            LocalLibrary::Owned(parts) => parts.info(),
            LocalLibrary::Borrowed(info) => info,
        }
    }
}

impl From<crate::library::kind::LibraryParts> for LocalLibrary<'_> {
    fn from(parts: crate::library::kind::LibraryParts) -> Self {
        LocalLibrary::Owned(Box::new(parts))
    }
}

impl From<Box<crate::library::kind::LibraryParts>> for LocalLibrary<'_> {
    fn from(parts: Box<crate::library::kind::LibraryParts>) -> Self {
        LocalLibrary::Owned(parts)
    }
}

impl<'a, 'b: 'a> From<&'a LibraryInfo<'b>> for LocalLibrary<'a> {
    fn from(info: &'a LibraryInfo<'b>) -> LocalLibrary<'a> {
        LocalLibrary::Borrowed(info)
    }
}

impl<'a> From<&'a crate::library::kind::LibraryParts> for LocalLibrary<'a> {
    fn from(parts: &'a crate::library::kind::LibraryParts) -> LocalLibrary<'a> {
        LocalLibrary::Borrowed(parts.info())
    }
}

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
/// The symbol is `no_mangle`, so the crate calling this is a `cdylib` and
/// nothing else; [`providers!`](crate::providers) says why.
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
            let ::core::option::Option::Some(host) = host else {
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
/// library knows. See [`LibraryInfo::unload`]. The expression is the
/// caller's own code and gets no `unsafe` from the macro:
///
/// ```compile_fail
/// use greeter_kind::Counter;
/// #[derive(Default, guatiao::Provider)]
/// #[provider(Counter)]
/// pub struct C;
/// impl Counter for C {
///     fn count(&self) -> i64 { 0 }
/// }
/// guatiao::providers!(
///     id = "leak",
///     version = "1",
///     providers = [C],
///     unload = *(16usize as *const unsafe extern "C" fn() -> guatiao::Status)
/// );
/// ```
///
/// # Call it from a crate that is only a `cdylib`
///
/// The two symbols this writes are `no_mangle`, and a `cdylib` exports
/// every `no_mangle` symbol of the Rust libraries linked into it. A crate
/// that calls this and is an `rlib` as well therefore makes each `cdylib`
/// depending on it scan and load as a library offering these providers.
/// Two such dependencies are a link error (`LNK2005` on MSVC, a multiple
/// definition elsewhere), or, where the linker never pulls one
/// dependency's object in, the other's entry symbol wins and nothing
/// reports it.
///
/// A provider that is also an ordinary Rust library is two crates: the
/// library derives `Provider` and exports the type, and a plugin crate of
/// one line names it.
///
/// ```ignore
/// // acme_plugin/src/lib.rs, crate-type = ["cdylib"], depending on `acme`
/// guatiao::providers!(acme::Tuner, acme::Recorder);
/// ```
///
/// The library's id and version are then the plugin crate's, and each
/// provider's default id is still `{package}_{type}` of the crate that
/// derived it: `acme_tuner` in a library `acme_plugin`.
#[macro_export]
macro_rules! providers {
    ($($provider:ty),+ $(,)?) => {
        $crate::providers!(
            id = ::core::env!("CARGO_PKG_NAME"),
            version = ::core::env!("CARGO_PKG_VERSION"),
            providers = [$($provider),+]
        );
    };
    ($($provider:ty),+ ; declares = [$($pair:expr),* $(,)?]) => {
        $crate::providers!(
            id = ::core::env!("CARGO_PKG_NAME"),
            version = ::core::env!("CARGO_PKG_VERSION"),
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
            fn __guatiao_describe -> &'static $crate::library::LibraryInfo<'static>
                = $crate::library::kind::LibraryParts::info;
            id = $id, version = $version, providers = [$($provider),+],
            unload = ::core::option::Option::Some(__guatiao_unload)
        );
        $crate::__guatiao_unload!(
            providers = [$($provider),+],
            then = {
                // Bound outside the `unsafe` block, so the caller's
                // expression gets none of it.
                let unload: unsafe extern "C" fn() -> $crate::Status = $unload;
                // SAFETY: the library's own slot, called as the host would.
                unsafe { unload() }
            }
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
            fn __guatiao_describe -> &'static $crate::library::LibraryInfo<'static>
                = $crate::library::kind::LibraryParts::info;
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
        // Safe to call, so its body is no `unsafe` context for `$then`;
        // it coerces to the slot's `unsafe extern "C" fn` type.
        #[doc(hidden)]
        extern "C" fn __guatiao_unload() -> $crate::Status {
            $crate::library::kind::catch(|| {
                let mut seen = $crate::__private::alloc::vec::Vec::new();
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
/// `library` builds the descriptor on each call and hands it over as
/// [`LibraryParts`](crate::library::kind::LibraryParts), which the
/// registry owns and frees when the library is retired: nothing is kept
/// for the process but each provider's default instance and tables,
/// which are the derive's and live in `static`s of their own.
#[macro_export]
macro_rules! local_providers {
    ($($provider:ty),+ $(,)?) => {
        $crate::local_providers!(
            id = ::core::env!("CARGO_PKG_NAME"),
            version = ::core::env!("CARGO_PKG_VERSION"),
            providers = [$($provider),+]
        );
    };
    (
        id = $id:expr,
        version = $version:expr,
        providers = [$($provider:ty),+ $(,)?] $(,)?
    ) => {
        /// What this crate offers as a library the host links: hand it to
        /// `Registry::register_local`, which owns what it builds.
        pub fn library(
            host: $crate::library::Host,
        ) -> ::core::option::Option<$crate::library::kind::LibraryParts> {
            // The host's arena when it offers one, as a loaded library's.
            let alloc = host.alloc().unwrap_or_else($crate::Alloc::rust);
            let mut providers = $crate::__private::alloc::vec::Vec::new();
            $(
                providers.push(
                    <$provider as $crate::library::kind::ProviderDecl>::provider(host, alloc).ok()?,
                );
            )+
            ::core::option::Option::Some(
                $crate::library::kind::LibraryParts::new($id, $version, providers),
            )
        }
    };
}

/// Writes a source file from a [`Source`](crate::library::Source) the
/// crate implements: the `providers!` of sources.
///
/// `serves` names what it serves, declared as `source=<serves>` so a scan
/// reports the file as a source and never loads it. `make` is a function
/// taking the [`Host`] and the configuration map the host passed to
/// `add_source_file` (empty when it passed none) and answering the source,
/// or a [`ProviderError`](crate::library::ProviderError) the host reads as
/// the refusal:
///
/// ```ignore
/// fn start(host: Host, config: &Map) -> Result<Shelf, ProviderError> { .. }
/// guatiao::source!(serves = "shelf", make = start);
/// ```
///
/// The source and the table it answers are kept for the life of the
/// process, as a source file's are. A descriptor `open` answers stays
/// valid until the registry closes it: owned parts are kept and dropped
/// before [`Source::close`](crate::library::Source::close) is called; a
/// borrowed descriptor is the source's to keep until then. A panic in any
/// call becomes a status. Call it from a crate that is only a `cdylib`,
/// for the reason [`providers!`](crate::providers) gives.
#[macro_export]
macro_rules! source {
    (serves = $serves:literal, make = $make:path $(,)?) => {
        $crate::__guatiao_declares!(kinds = [], pairs = [::core::concat!("source=", $serves)]);
        /// The symbol a `guatiao` host adds this source by.
        ///
        /// # Safety
        ///
        /// As `guatiao_source` in `guatiao.h`: `host` is null or a host
        /// descriptor valid for the process, `config` null or a
        /// well-formed value, `out` and `err` null or writable.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn guatiao_source(
            host: *const $crate::library::HostInfo<'static>,
            config: *const $crate::Value,
            out: *mut *const $crate::library::SourceInfo,
            err: *mut $crate::library::ProviderError,
        ) -> $crate::Status {
            // Bound outside the `unsafe` block, so the caller's path gets
            // none of it.
            let make = $make;
            // SAFETY: the loader's side of the contract, forwarded.
            unsafe { $crate::library::source_entry(host, config, out, err, make) }
        }
    };
}

/// The describe function behind [`providers!`](crate::providers): builds
/// the descriptor once, for as long as the library is mapped, and answers
/// it through `$answer`.
#[doc(hidden)]
#[macro_export]
macro_rules! __guatiao_describe {
    (
        $(#[$attr:meta])* $vis:vis fn $name:ident -> $ret:ty = $answer:path;
        id = $id:expr, version = $version:expr, providers = [$($provider:ty),+],
        unload = $unload:expr
    ) => {
        $(#[$attr])*
        $vis fn $name(
            host: $crate::library::Host,
        ) -> ::core::option::Option<$ret> {
            static REGISTERED: ::std::sync::OnceLock<::core::option::Option<$crate::library::kind::LibraryParts>> = ::std::sync::OnceLock::new();
            REGISTERED
                .get_or_init(|| {
                    // The host's arena when it offers one, so what this
                    // library builds for the host outlives this mapping.
                    let alloc = host.alloc().unwrap_or_else($crate::Alloc::rust);
                    let mut providers = $crate::__private::alloc::vec::Vec::new();
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
                .map($answer)
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
        Ok(None) => core::ptr::null(),
        Err(payload) => {
            // Dropping a panic payload can panic, and a second panic
            // inside an `extern "C"` body is an abort.
            core::mem::forget(payload);
            core::ptr::null()
        }
    }
}
