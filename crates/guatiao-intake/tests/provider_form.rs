// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A provider's form travels in its descriptor: `#[provider(.., form =
//! Ty)]` writes `Ty`'s screen into `meta["form"]`, and a host reads it
//! back from the offer, beside the configuration schema it presents.

#![cfg(feature = "derive")]

use guatiao::Alloc;
use guatiao::library::Registry;
use guatiao_intake::{Form, FormRef, PROVIDER_FORM_KEY, Screen};

#[guatiao::kind]
pub trait Dial: Send + Sync {
    /// How many rings.
    fn rings(&self) -> i64;
}

/// What the dialer is configured with, and how a person is shown it.
#[derive(guatiao::Schema, guatiao::FromValue, Form)]
#[form(section(id = "net", label = "Network"))]
pub struct Settings {
    /// Where to dial.
    #[schema(section = "net")]
    #[form(widget = "hostname", placeholder = "pbx.example")]
    host: String,
}

/// A provider built from `Settings`, presented by `Settings`' screen.
#[derive(guatiao::Provider)]
#[provider(Dial, config = Settings, form = Settings)]
pub struct Dialer {
    #[allow(dead_code)]
    host: String,
}

impl From<Settings> for Dialer {
    fn from(settings: Settings) -> Dialer {
        Dialer {
            host: settings.host,
        }
    }
}

impl Dial for Dialer {
    fn rings(&self) -> i64 {
        3
    }
}

/// No form: no `meta` at all.
#[derive(Default, guatiao::Provider)]
#[provider(Dial)]
pub struct Plain;

impl Dial for Plain {
    fn rings(&self) -> i64 {
        1
    }
}

guatiao::local_providers!(id = "forms", version = "1.0.0", providers = [Dialer, Plain]);

fn registry() -> Registry {
    let mut registry = Registry::new("form-tests", "1.0");
    registry
        .register_local("forms", library)
        .expect("the linked library registers")
        .loaded()
        .expect("it accepts this host");
    registry
}

#[test]
fn a_provider_s_form_is_read_back_from_its_meta() {
    let registry = registry();
    let offer = registry
        .offer::<dyn Dial>("guatiao_intake_dialer")
        .expect("filed under its id")
        .expect("a valid table");
    let form = offer
        .meta()
        .and_then(|meta| meta.get(PROVIDER_FORM_KEY))
        .expect("the descriptor carries a form under `form`");
    assert_eq!(
        *form,
        <Settings as Screen>::form(Alloc::rust()).unwrap(),
        "the screen `Settings` declares, as built"
    );
    let form = FormRef::new(form).expect("a form value");
    let sections: Vec<(String, String)> = form
        .sections()
        .map(|s| (s.id().to_string(), s.label().to_string()))
        .collect();
    assert_eq!(sections, [("net".to_string(), "Network".to_string())]);
    let hints = form.hints("host");
    assert_eq!(hints.widget(), "hostname");
    assert_eq!(hints.placeholder(), "pbx.example");

    let plain = registry
        .offer::<dyn Dial>("guatiao_intake_plain")
        .expect("filed")
        .expect("valid");
    assert!(plain.meta().is_none(), "no form, no meta");
}
