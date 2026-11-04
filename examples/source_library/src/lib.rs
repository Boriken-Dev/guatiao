// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source written in Rust: one greeter library per entry of its
//! configuration's `greetings` map (`{"hi": "hello"}` serves a library
//! `hi` whose provider `hi_greeter` greets with "hello"), and the words of
//! its optional `note`, told as a note about the whole source with the
//! detail `{"said": "by Rust"}`. The source is a
//! [`Source`] like any a Rust host links; `guatiao::source!` makes this
//! file one any host adds, in any language.

extern crate alloc;

use alloc::sync::Arc;

use greeter_kind::{Greeter, GreeterVtable};
use guatiao::library::kind::{LibraryParts, ProviderParts};
use guatiao::library::{Candidate, Host, KindTable, LocalLibrary, Note, ProviderError, Source};
use guatiao::value::convert::TryAsRef;
use guatiao::{Map, Status};

/// One library's greeter: its word.
struct Hello {
    word: String,
}

impl Greeter for Hello {
    fn greet(&self, name: &str) -> Result<Map, ProviderError> {
        if name.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "nobody to greet",
            ));
        }
        let mut answer = Map::new();
        answer.set("greeting", format!("{}, {name}", self.word).as_str());
        Ok(answer)
    }
}

static TABLE: GreeterVtable = GreeterVtable::of::<Hello>();

/// The libraries the configuration names, and their words.
struct Shelf {
    greetings: Vec<(String, String)>,
    note: Option<String>,
}

impl Source for Shelf {
    fn list(&self, _host: Host) -> Result<Vec<Candidate>, ProviderError> {
        Ok(self
            .greetings
            .iter()
            .map(|(id, word)| {
                Candidate::new(id.as_str(), "1.0.0", vec!["greeter".into()], word.as_str())
            })
            .collect())
    }

    fn open(&self, _host: Host, id: &str) -> Result<LocalLibrary<'static>, ProviderError> {
        let (_, word) = self
            .greetings
            .iter()
            .find(|(known, _)| known == id)
            .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_NOT_FOUND, "no such library"))?;
        // The greeter the table's `ctx` names, kept with the provider.
        let hello = Arc::new(Hello { word: word.clone() });
        let provider = ProviderParts::builder(&format!("{id}_greeter"))
            .kinds(&["greeter"])
            .table(KindTable::new(
                "greeter",
                (&TABLE as *const GreeterVtable).cast(),
                size_of::<GreeterVtable>(),
            ))
            .ctx(Arc::as_ptr(&hello).cast_mut().cast())
            .keep(hello)
            .build();
        Ok(LibraryParts::new(id, "1.0.0", vec![provider]).into())
    }

    fn notes(&self) -> Result<Vec<Note>, ProviderError> {
        let mut detail = Map::new();
        detail.set("said", "by Rust");
        Ok(self
            .note
            .iter()
            .map(|words| Note::new("", words.as_str()).with_detail(detail.clone()))
            .collect())
    }
}

/// Reads `{"greetings": {<id>: <word>, ..}, "note"?: <words>}`.
fn start(_host: Host, config: &Map) -> Result<Shelf, ProviderError> {
    let greetings = config
        .get("greetings")
        .and_then(TryAsRef::<Map>::try_as_ref)
        .ok_or_else(|| {
            ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "`greetings` is missing or not a map",
            )
        })?;
    let greetings = greetings
        .iter()
        .map(|entry| {
            let word = TryAsRef::<str>::try_as_ref(entry.value()).ok_or_else(|| {
                ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, "a greeting is not text")
            })?;
            Ok((entry.key().to_string(), word.to_string()))
        })
        .collect::<Result<_, ProviderError>>()?;
    let note = match config.get("note") {
        None => None,
        Some(words) => Some(
            TryAsRef::<str>::try_as_ref(words)
                .filter(|words| !words.is_empty())
                .ok_or_else(|| {
                    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, "`note` is not words")
                })?
                .to_string(),
        ),
    };
    Ok(Shelf { greetings, note })
}

guatiao::source!(serves = "greeters", make = start);
