// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! How one library offers any number of providers to a host that has
//! never heard of it.
//!
//! # One symbol, and an envelope that defines no vtable
//!
//! A library exports exactly `guatiao_library_entry`. The host calls it
//! with a description of itself and gets back a description of the
//! library: an id, a version, and a list of providers. Each provider says
//! what **kind** of thing it is, what its id is, what configuration it
//! takes (as an ordinary schema value), and carries an opaque pointer to
//! its function table plus the size that table was compiled at.
//!
//! The envelope never says what a table looks like. Whoever defines a
//! kind defines that, which is what lets one mechanism carry a codec, a
//! credential store and a greeter without knowing anything about any of
//! them.
//!
//! # Nothing here is a registry a library writes into
//!
//! The library is **asked** and answers. [`Registry`] is a value the host
//! owns, so two artifacts in one process that each link this crate hold
//! two of them and neither can be surprised by the other. It is also why
//! the failure mode of a callback-registration design cannot arise here:
//! a library registering into a table the host never reads, where the
//! symptom is a lookup answering "not found" for something that was
//! definitely registered.
//!
//! # A loaded library is never unloaded
//!
//! Everything a library hands over points into its mapping: the
//! descriptor, the text inside it, the vtables, and the allocator
//! recorded inside every tree it builds. Unloading invalidates all of
//! them at once, and the first symptom would be a free through an
//! unmapped function pointer at teardown. So the handle is forgotten and
//! the mapping stays — which is the same rule [`crate::value::Alloc`]
//! states: an allocator outlives everything built through it.
//!
//! # Writing a library
//!
//! ```ignore
//! use guatiao::library::{HostInfo, LibraryInfo};
//!
//! fn describe(host: &HostInfo) -> Option<&'static LibraryInfo> {
//!     // build the descriptor once, keep it in a `OnceLock` of your own
//! }
//! guatiao::guatiao_library!(describe);
//! ```
//!
//! A `OnceLock` inside a library is fine. The rule against process-global
//! state binds *this crate*, so that a host and a library can each link
//! their own copy of it with nothing to disagree about; a library holding
//! its own descriptor is not that.

pub mod desc;
// What a host files a provider under. A `//` comment, never a `///`.
pub mod key;
// The one file here with `unsafe` in it: raw descriptor reads, the entry
// macro, and the loader. A `//` comment, never a `///`.
pub mod raw;
// Loading is behind the `load` feature, so a library AUTHOR — who needs
// the descriptors and the macro and never loads anything — builds with
// no dependency at all. A `//` comment, never a `///`.
#[cfg(feature = "load")]
pub mod registry;
// Finding libraries without running them. Behind `load` with the loader
// it feeds. A `//` comment, never a `///`.
#[cfg(feature = "load")]
pub mod scan;

pub use desc::{ABI_VERSION, HostInfo, LibraryInfo, ProviderInfo, Providers};
pub use key::{KeyError, KeyFields, KeyTemplate};
pub use raw::{ENTRY_SYMBOL, EntryFn, LibraryView, ProviderView, answer, read_host};
#[cfg(feature = "load")]
pub use registry::{LoadError, Loaded, Provider, Registry, Skipped};
#[cfg(feature = "load")]
pub use scan::{LoadReport, declares_entry_symbol, scan_dir};
