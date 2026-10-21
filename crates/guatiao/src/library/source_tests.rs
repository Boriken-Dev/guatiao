// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source a Rust host links, refreshed through every change a source
//! can make. No library is mapped, so these run under Miri.

#![forbid(unsafe_code)]

use std::sync::Mutex;

use super::*;
use crate::library::kind::{LibraryParts, ProviderParts};

/// A shelf of libraries: what it lists is what the test put there, and
/// it counts every open and close.
#[derive(Default)]
struct Shelf {
    listed: Mutex<Vec<(String, String)>>,
    refusing: Mutex<Option<String>>,
    unlistable: Mutex<bool>,
    /// The id a candidate's library is opened under, when it is not its own.
    calls_itself: Mutex<Option<(String, String)>>,
    opened: Mutex<Vec<(String, String)>>,
    closed: Mutex<Vec<(String, String)>>,
}

impl Shelf {
    fn put(&self, id: &str, stamp: &str) {
        let mut listed = self.listed.lock().unwrap();
        listed.retain(|(i, _)| i != id);
        listed.push((id.to_string(), stamp.to_string()));
    }

    fn take(&self, id: &str) {
        self.listed.lock().unwrap().retain(|(i, _)| i != id);
    }

    fn closed(&self) -> Vec<(String, String)> {
        self.closed.lock().unwrap().clone()
    }
}

/// The source the registry holds: a shared shelf the test keeps changing.
struct OnShelf(Arc<Shelf>);

impl Source for OnShelf {
    fn list(&self, _host: Host) -> Result<Vec<Candidate>, ProviderError> {
        if *self.0.unlistable.lock().unwrap() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the shelf fell over",
            ));
        }
        Ok(self
            .0
            .listed
            .lock()
            .unwrap()
            .iter()
            .map(|(id, stamp)| {
                Candidate::new(id.as_str(), "1.0.0", vec!["thing".into()], stamp.as_str())
            })
            .collect())
    }

    fn open(&self, _host: Host, id: &str) -> Result<LocalLibrary<'static>, ProviderError> {
        if let Some(why) = self.0.refusing.lock().unwrap().clone() {
            return Err(ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why));
        }
        let stamp = self
            .0
            .listed
            .lock()
            .unwrap()
            .iter()
            .find(|(i, _)| i == id)
            .map(|(_, s)| s.clone())
            .unwrap_or_default();
        self.0
            .opened
            .lock()
            .unwrap()
            .push((id.to_string(), stamp.clone()));
        let named = match self.0.calls_itself.lock().unwrap().clone() {
            Some((listed, actual)) if listed == id => actual,
            _ => id.to_string(),
        };
        let provider = ProviderParts::builder(&format!("{named}_p"))
            .version(&format!("1.0.{stamp}"))
            .kinds(&["thing"])
            .build();
        Ok(LibraryParts::new(&named, "1.0.0", vec![provider]).into())
    }

    fn close(&self, id: &str, stamp: &str) {
        self.0
            .closed
            .lock()
            .unwrap()
            .push((id.to_string(), stamp.to_string()));
    }
}

fn a_registry_with(shelf: &Arc<Shelf>) -> Registry {
    let mut registry = Registry::new("source-tests", "1.0");
    registry
        .add_source("shelf", OnShelf(Arc::clone(shelf)))
        .expect("the first source by that name");
    registry
}

fn ids(served: &[Served]) -> Vec<(&str, &str)> {
    served
        .iter()
        .map(|s| (s.id.as_str(), s.stamp.as_str()))
        .collect()
}

fn pair(id: &str, stamp: &str) -> (String, String) {
    (id.to_string(), stamp.to_string())
}

#[test]
fn a_refresh_loads_what_the_source_lists_and_nothing_twice() {
    let shelf = Arc::new(Shelf::default());
    shelf.put("a", "1");
    shelf.put("b", "1");
    let mut registry = a_registry_with(&shelf);
    assert!(
        registry.loaded().is_empty(),
        "nothing opens before a refresh"
    );

    let first = registry.refresh();
    assert_eq!(ids(&first.loaded), [("a", "1"), ("b", "1")]);
    assert_eq!(first.loaded[0].source, "shelf");
    assert_eq!(first.loaded[0].key, "a");
    assert_eq!(registry.providers("thing").count(), 2);
    assert_eq!(registry.loaded()[0].path, PathBuf::from("<shelf:a>"));

    assert!(registry.refresh().is_empty(), "nothing changed");
    assert_eq!(shelf.opened.lock().unwrap().len(), 2);
    assert_eq!(registry.sources(), ["shelf"]);
}

#[test]
fn a_vanished_candidate_is_retired_and_closed_exactly_once() {
    let shelf = Arc::new(Shelf::default());
    shelf.put("a", "1");
    shelf.put("b", "1");
    let mut registry = a_registry_with(&shelf);
    registry.refresh();

    shelf.take("b");
    let gone = registry.refresh();
    assert_eq!(ids(&gone.retired), [("b", "1")]);
    assert_eq!(gone.retired[0].key, "b");
    assert!(registry.provider("b_p").is_none());
    // Nothing held it: the retire's own collection closed it.
    registry.collect();
    assert_eq!(shelf.closed(), [pair("b", "1")]);
    registry.refresh();
    registry.collect();
    assert_eq!(shelf.closed(), [pair("b", "1")], "closed once");
}

#[test]
fn a_changed_candidate_serves_the_new_build_while_the_old_one_is_held() {
    let shelf = Arc::new(Shelf::default());
    shelf.put("a", "1");
    let mut registry = a_registry_with(&shelf);
    registry.refresh();
    let held = registry.provider("a_p").expect("loaded").view().clone();
    assert_eq!(held.version(), Some("1.0.1"));

    shelf.put("a", "2");
    let changed = registry.refresh();
    assert_eq!(ids(&changed.loaded), [("a", "2")]);
    assert_eq!(ids(&changed.retired), [("a", "1")]);
    assert_eq!(
        registry
            .provider("a_p")
            .expect("the new build")
            .view()
            .version(),
        Some("1.0.2"),
        "a new lookup finds the new build"
    );
    registry.collect();
    assert!(shelf.closed().is_empty(), "the old build is held");

    drop(held);
    assert_eq!(registry.collect(), ["a"]);
    assert_eq!(
        shelf.closed(),
        [pair("a", "1")],
        "the old build, by its stamp"
    );
    registry.collect();
    assert_eq!(shelf.closed(), [pair("a", "1")], "closed once");
}

#[test]
fn a_refused_open_is_reported_and_the_old_build_keeps_serving() {
    let shelf = Arc::new(Shelf::default());
    shelf.put("a", "1");
    let mut registry = a_registry_with(&shelf);
    registry.refresh();

    *shelf.refusing.lock().unwrap() = Some("not today".to_string());
    shelf.put("a", "2");
    let refused = registry.refresh();
    assert!(refused.loaded.is_empty() && refused.retired.is_empty());
    assert_eq!(refused.refused.len(), 1);
    assert_eq!(refused.refused[0].id.as_deref(), Some("a"));
    assert_eq!(refused.refused[0].reason, "not today");
    assert_eq!(
        registry.provider("a_p").unwrap().view().version(),
        Some("1.0.1")
    );

    // Not tried again until it changes.
    *shelf.refusing.lock().unwrap() = None;
    assert!(registry.refresh().is_empty());
    shelf.put("a", "3");
    assert_eq!(ids(&registry.refresh().loaded), [("a", "3")]);
    registry.collect();
    assert_eq!(
        shelf.closed(),
        [pair("a", "1")],
        "a refused open is never closed"
    );
}

#[test]
fn an_open_that_is_not_the_candidate_is_refused_and_closed() {
    let shelf = Arc::new(Shelf::default());
    shelf.put("a", "1");
    *shelf.calls_itself.lock().unwrap() = Some(("a".to_string(), "impostor".to_string()));
    let mut registry = a_registry_with(&shelf);
    let refused = registry.refresh();
    assert!(refused.loaded.is_empty());
    assert!(
        refused.refused[0].reason.contains("impostor"),
        "{refused:?}"
    );
    assert_eq!(
        shelf.closed(),
        [pair("a", "1")],
        "opened, not kept, closed at once"
    );
}

#[test]
fn a_library_the_host_retired_stays_out_until_it_changes() {
    let shelf = Arc::new(Shelf::default());
    shelf.put("a", "1");
    let mut registry = a_registry_with(&shelf);
    registry.refresh();
    registry.retire("a").expect("the host's own retire");
    assert!(registry.refresh().is_empty(), "not brought back");
    assert!(registry.provider("a_p").is_none());
    shelf.put("a", "2");
    assert_eq!(ids(&registry.refresh().loaded), [("a", "2")]);
}

#[test]
fn a_source_that_cannot_list_is_reported_and_keeps_what_it_served() {
    let shelf = Arc::new(Shelf::default());
    shelf.put("a", "1");
    let mut registry = a_registry_with(&shelf);
    registry.refresh();
    *shelf.unlistable.lock().unwrap() = true;
    let report = registry.refresh();
    assert_eq!(report.refused.len(), 1);
    assert_eq!(report.refused[0].id, None);
    assert_eq!(report.refused[0].reason, "the shelf fell over");
    assert!(registry.provider("a_p").is_some());
}

#[test]
fn a_second_source_of_one_name_is_refused() {
    let shelf = Arc::new(Shelf::default());
    let mut registry = a_registry_with(&shelf);
    let again = registry.add_source("shelf", OnShelf(Arc::clone(&shelf)));
    assert!(
        matches!(again, Err(SourceError::Duplicate { .. })),
        "{again:?}"
    );
}

#[test]
fn a_served_library_unloads_once_nothing_holds_it() {
    let shelf = Arc::new(Shelf::default());
    shelf.put("a", "1");
    let mut registry = a_registry_with(&shelf);
    registry.refresh();
    let held = registry.provider("a_p").unwrap().view().clone();
    assert!(matches!(
        registry.unload("a"),
        Err(crate::library::UnloadError::Busy { .. })
    ));
    drop(held);
    registry
        .unload("a")
        .expect("nothing holds it, and its source's close is its promise");
    assert_eq!(shelf.closed(), [pair("a", "1")]);
}
