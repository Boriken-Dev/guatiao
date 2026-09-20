// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Legal notices a library or a provider states, as a convention on
//! `meta`.
//!
//! # Why a convention on `meta` and not a descriptor field
//!
//! A notice is text a host must be able to SHOW — some licences require
//! the attribution to be displayed, which no file in a repository can
//! satisfy — and it is per library, discovered at run time, because only
//! the library knows what it links. That is exactly what `meta` is for:
//! a thing this envelope did not think of, carried as an ordinary value a
//! reader that does not know the key skips. Naming the key here, rather
//! than leaving each host to invent one, is what lets a host list every
//! notice it owes without knowing any kind.
//!
//! # The shape
//!
//! `meta["notices"]` is a list of maps, one per licence:
//!
//! ```text
//! {"component": "openh264", "text": "OpenH264 Video Codec provided by ...", "format": "text"}
//! ```
//!
//! - `component` names what the licence applies to. Empty means the
//!   library (or provider) as a whole, and a host shows its display name
//!   instead — one library can carry several third-party licences, so
//!   the subject is on the notice, not on the library.
//! - `text` is the notice itself. Blank contributes nothing at all: not a
//!   heading over an empty body, not boilerplate.
//! - `format` is **declared, never sniffed**: [`FORMAT_TEXT`] or
//!   [`FORMAT_MARKDOWN`]. Rendering a licence written as prose through a
//!   Markdown renderer mangles it — hard-wrapped lines with leading
//!   indentation become code blocks, `*` and `_` become emphasis — and a
//!   notice that has been reflowed is a notice that was not reproduced.
//!   So anything that is not `markdown`, an absent format included,
//!   reads as text, which is the safe direction: an unrendered notice is
//!   still a disclosure.
//!
//! # Read on display, never cached
//!
//! A library that loads an optional component on demand owes that
//! component's attribution only while it is in use, and `meta` is a
//! pointer the library keeps: what it holds may change over a process's
//! life. A host reads the notices when it is about to show them.

#![forbid(unsafe_code)]

use crate::value::ValueError;
use crate::value::convert::TryAsRef;
use crate::value::types::{List, Map, Text, Value};

/// The key under `meta` that holds the list.
pub const NOTICES_KEY: &str = "notices";
/// The key on one notice naming what the licence applies to.
pub const COMPONENT_KEY: &str = "component";
/// The key on one notice holding the text.
pub const TEXT_KEY: &str = "text";
/// The key on one notice declaring how the text is written.
pub const FORMAT_KEY: &str = "format";
/// The text is prose, reproduced verbatim. The default, and the reading
/// of any format a host does not recognise.
pub const FORMAT_TEXT: &str = "text";
/// The text is Markdown, and a host may render it.
pub const FORMAT_MARKDOWN: &str = "markdown";

/// One legal notice: what it covers, its text, and how the text is
/// written. Borrowed from the `meta` it was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notice<'a> {
    /// What this licence applies to; empty means the whole library or
    /// provider, and a host substitutes the display name.
    pub component: &'a str,
    /// The notice itself. Never blank: a blank one is not returned.
    pub text: &'a str,
    /// [`FORMAT_TEXT`], [`FORMAT_MARKDOWN`], or whatever the library
    /// wrote; [`Notice::is_markdown`] is the one question to ask of it.
    pub format: &'a str,
}

impl<'a> Notice<'a> {
    /// A notice written as prose.
    pub const fn text(component: &'a str, text: &'a str) -> Notice<'a> {
        Notice {
            component,
            text,
            format: FORMAT_TEXT,
        }
    }

    /// A notice written as Markdown.
    pub const fn markdown(component: &'a str, text: &'a str) -> Notice<'a> {
        Notice {
            component,
            text,
            format: FORMAT_MARKDOWN,
        }
    }

    /// Whether a host may render the text as Markdown. Anything but an
    /// explicit `markdown` is prose.
    pub fn is_markdown(&self) -> bool {
        self.format == FORMAT_MARKDOWN
    }
}

/// Every notice `meta` declares, in the order written; empty for no
/// `meta`, no list, or a list holding nothing usable.
///
/// A blank text is dropped; a missing or unrecognised format is kept as
/// written and reads as text through [`Notice::is_markdown`]; an entry
/// that is not a map is skipped. Nothing here is an error: the list is
/// what a library wrote, and a host shows what it can.
pub fn notices(meta: Option<&Map>) -> Vec<Notice<'_>> {
    let Some(list) = meta
        .and_then(|m| m.get(NOTICES_KEY))
        .and_then(TryAsRef::<List>::try_as_ref)
    else {
        return Vec::new();
    };
    list.iter()
        .filter_map(TryAsRef::<Map>::try_as_ref)
        .filter_map(|one| {
            let text = str_of(one, TEXT_KEY)?;
            if text.trim().is_empty() {
                return None;
            }
            Some(Notice {
                component: str_of(one, COMPONENT_KEY).unwrap_or(""),
                text,
                format: str_of(one, FORMAT_KEY).unwrap_or(FORMAT_TEXT),
            })
        })
        .collect()
}

/// The text under `key`, or `None` for an absent key or another kind.
fn str_of<'a>(one: &'a Map, key: &str) -> Option<&'a str> {
    one.get(key).and_then(TryAsRef::<str>::try_as_ref)
}

/// Appends `notice` to `meta`'s list, creating the list on the first
/// call, through the allocator `meta` records.
///
/// What a library calls while building the `meta` it will hand out. A
/// blank text is refused with [`ValueError::WrongKind`] rather than
/// stored -- it is not a notice -- so a host never has to decide what an
/// empty one means.
pub fn declare_notice(meta: &mut Map, notice: Notice<'_>) -> Result<(), ValueError> {
    if notice.text.trim().is_empty() {
        return Err(ValueError::WrongKind);
    }
    let alloc = meta.alloc()?;
    let mut one = Map::new_in(alloc);
    one.set(
        COMPONENT_KEY,
        Text::new_in(alloc, notice.component).map(Value::from)?,
    )?;
    one.set(TEXT_KEY, Text::new_in(alloc, notice.text).map(Value::from)?)?;
    one.set(
        FORMAT_KEY,
        Text::new_in(alloc, notice.format).map(Value::from)?,
    )?;

    meta.push_into(NOTICES_KEY, one)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_declared_is_no_notices() {
        assert!(notices(None).is_empty());
        assert!(notices(Some(&Map::new())).is_empty());
        let mut meta = Map::new();
        meta.set(NOTICES_KEY, Value::from(Text::new("not a list")))
            .unwrap();
        assert!(
            notices(Some(&meta)).is_empty(),
            "a list is the only shape read"
        );
    }

    #[test]
    fn declared_notices_read_back_in_order_with_their_format() {
        let mut meta = Map::new();
        declare_notice(
            &mut meta,
            Notice::text("openh264", "OpenH264 provided by Cisco"),
        )
        .unwrap();
        declare_notice(&mut meta, Notice::markdown("", "# GPL\n\nsee COPYING")).unwrap();

        let read = notices(Some(&meta));
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].component, "openh264");
        assert_eq!(read[0].text, "OpenH264 provided by Cisco");
        assert!(!read[0].is_markdown());
        assert_eq!(read[1].component, "", "empty means the whole library");
        assert!(read[1].is_markdown());
    }

    #[test]
    fn a_blank_text_is_refused_on_write_and_dropped_on_read() {
        let mut meta = Map::new();
        assert_eq!(
            declare_notice(&mut meta, Notice::text("x", "   \n")),
            Err(ValueError::WrongKind)
        );
        assert!(meta.get(NOTICES_KEY).is_none(), "a refusal writes nothing");

        // A list somebody else wrote, holding a blank entry, an entry
        // that is not a map, and one with an unknown format.
        let mut list = List::new();
        let mut blank = Map::new();
        blank.set(TEXT_KEY, "   ").unwrap();
        list.push(blank).unwrap();
        list.push(Value::from(7i64)).unwrap();
        let mut odd = Map::new();
        odd.set(TEXT_KEY, "some text").unwrap();
        odd.set(FORMAT_KEY, "rtf").unwrap();
        list.push(odd).unwrap();
        meta.set(NOTICES_KEY, list).unwrap();

        let read = notices(Some(&meta));
        assert_eq!(read.len(), 1, "the blank and the non-map are dropped");
        assert_eq!(read[0].format, "rtf", "kept as written");
        assert!(
            !read[0].is_markdown(),
            "and read as text, the safe direction"
        );
    }

    #[test]
    fn the_text_is_reproduced_verbatim() {
        // Leading indentation and trailing whitespace are part of a
        // licence's text; trimming decides emptiness only.
        let mut meta = Map::new();
        let text = "    Copyright (c) 2026\n   All rights reserved.  \n";
        declare_notice(&mut meta, Notice::text("", text)).unwrap();
        assert_eq!(notices(Some(&meta))[0].text, text);
    }
}
