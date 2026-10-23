// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A daemon's configuration read through guatiao, with one set of
//! attributes: addresses, paths, an optional address, an enum told apart
//! by kind, a TOML table it does not describe, and lists and maps of
//! those, with no wrapper type anywhere. It derives its schema and form,
//! its own schema accepts what it writes, it reads from TOML text, and
//! intake's `check` accepts the form naming its fields.

extern crate alloc;

use alloc::collections::BTreeMap;
use core::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::schema::validate::validate_map;
use guatiao::{Alloc, FromValue, Map, MapError, Schema, ToValue};
use guatiao_intake::{Form, FormRef, Screen, check};
use guatiao_serde::Presentation;
use guatiao_serde::text::toml as toml_text;

/// The certificate a listener presents.
#[derive(ToValue, FromValue, Schema, Debug, PartialEq, Clone)]
#[map(untagged, rename_all = "lowercase")]
enum Tls {
    /// The machine's own identity.
    Identity,
    /// A certificate and its key, from files.
    Files { certificate: PathBuf, key: PathBuf },
}

#[derive(ToValue, FromValue, Schema, Debug, PartialEq, Clone)]
struct Upstream {
    address: SocketAddr,
    tls: Tls,
}

#[derive(ToValue, FromValue, Schema, Form, Debug, PartialEq)]
#[form(
    section(id = "net", label = "Network"),
    section(id = "store", label = "Storage")
)]
struct Daemon {
    /// Where to listen.
    #[schema(section = "net")]
    #[form(placeholder = "0.0.0.0:7000")]
    listen: SocketAddr,
    /// The address to advertise, when it is not the one listened on.
    #[schema(section = "net")]
    #[form(placeholder = "192.0.2.10")]
    advertise: Option<IpAddr>,
    /// Where state is kept.
    #[schema(section = "store")]
    #[form(placeholder = "/var/lib/daemon")]
    state: PathBuf,
    tls: Tls,
    peers: Vec<SocketAddr>,
    fallbacks: Vec<Tls>,
    upstreams: BTreeMap<String, Upstream>,
    /// Settings passed through to the runtime, which describes them.
    settings: toml::Table,
    plugins: BTreeMap<String, toml::Table>,
}

const DOCUMENT: &str = r#"
listen = "0.0.0.0:7000"
advertise = "192.0.2.10"
state = "/var/lib/daemon"
tls = "identity"
peers = ["10.0.0.2:7000", "[fe80::1%2]:7000"]
fallbacks = ["identity", { certificate = "/etc/ssl/f.pem", key = "/etc/ssl/f.key" }]

[upstreams.north]
address = "10.1.0.1:443"
tls = { certificate = "/etc/ssl/n.pem", key = "/etc/ssl/n.key" }

[upstreams.south]
address = "10.2.0.1:443"
tls = "identity"

[settings]
workers = 4
mode = "fast"

[plugins.scaler]
factor = 1.5

[plugins.scaler.limits]
max = 10
"#;

fn files(name: &str) -> Tls {
    Tls::Files {
        certificate: format!("/etc/ssl/{name}.pem").into(),
        key: format!("/etc/ssl/{name}.key").into(),
    }
}

fn expected() -> Daemon {
    let whole: toml::Table = DOCUMENT.parse().expect("a TOML document");
    let table = |key: &str| whole[key].as_table().cloned().expect("a table");
    Daemon {
        listen: "0.0.0.0:7000".parse().unwrap(),
        advertise: Some("192.0.2.10".parse().unwrap()),
        state: "/var/lib/daemon".into(),
        tls: Tls::Identity,
        peers: vec![
            "10.0.0.2:7000".parse().unwrap(),
            "[fe80::1%2]:7000".parse().unwrap(),
        ],
        fallbacks: vec![Tls::Identity, files("f")],
        upstreams: BTreeMap::from([
            (
                "north".to_string(),
                Upstream {
                    address: "10.1.0.1:443".parse().unwrap(),
                    tls: files("n"),
                },
            ),
            (
                "south".to_string(),
                Upstream {
                    address: "10.2.0.1:443".parse().unwrap(),
                    tls: Tls::Identity,
                },
            ),
        ]),
        settings: table("settings"),
        plugins: BTreeMap::from([(
            "scaler".to_string(),
            table("plugins")["scaler"].as_table().cloned().unwrap(),
        )]),
    }
}

fn read(text: &str) -> Result<Daemon, Box<dyn core::error::Error>> {
    let value = toml_text::from_str(text, Alloc::rust(), Presentation::new())?;
    Ok(Daemon::from_value(&value)?)
}

#[test]
fn it_reads_from_toml_text() {
    assert_eq!(read(DOCUMENT).expect("reads"), expected());
}

#[test]
fn its_own_schema_accepts_what_it_writes_and_the_document() {
    let schema = Daemon::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    validate_map(s, &expected().to_value().unwrap()).expect("accepts what it writes");
    let document = toml_text::from_str(DOCUMENT, Alloc::rust(), Presentation::new()).unwrap();
    validate_map(s, &document).expect("accepts the document");
    // And reads back what it wrote.
    let written = expected().to_value().unwrap();
    assert_eq!(Daemon::from_value(&written).unwrap(), expected());
}

#[test]
fn intake_checks_the_form_naming_its_fields() {
    let schema = Daemon::schema().unwrap();
    let form = Daemon::form(Alloc::rust()).unwrap();
    let form_ref = FormRef::new(&form).unwrap();
    let named: Vec<&str> = form_ref.fields().map(|(key, _)| key).collect();
    for key in ["listen", "advertise", "state"] {
        assert!(named.contains(&key), "the form names {key}: {named:?}");
    }
    check(SchemaRef::new(&schema).unwrap(), form_ref).expect("the form fits the schema");
}

#[test]
fn each_field_describes_itself() {
    let schema = Daemon::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let kind = |key: &str| s.find(key).unwrap().kind();
    assert!(matches!(
        kind("listen"),
        Kind::Str {
            pattern: Some(_),
            ..
        }
    ));
    assert!(matches!(kind("state"), Kind::Str { .. }));
    assert!(!s.find("advertise").unwrap().is_required());
    assert!(matches!(kind("tls"), Kind::Union(_)));
    assert!(matches!(kind("settings"), Kind::MapOf(_)));
    assert!(matches!(kind("upstreams"), Kind::MapOf(_)));
    assert!(matches!(kind("fallbacks"), Kind::List { .. }));
}

#[test]
fn a_mistake_is_named_by_its_key() {
    let wrong_tls = DOCUMENT.replace("tls = \"identity\"\npeers", "tls = 5\npeers");
    let error = read(&wrong_tls).unwrap_err();
    let error = error.downcast_ref::<MapError>().expect("a reading error");
    assert!(
        matches!(error, MapError::BadValue { key, .. } if key == "tls"),
        "{error:?}"
    );

    let wrong_peer = DOCUMENT.replace("\"10.0.0.2:7000\"", "\"10.0.0.2\"");
    let error = read(&wrong_peer).unwrap_err();
    let error = error.downcast_ref::<MapError>().expect("a reading error");
    assert!(
        matches!(error, MapError::BadValue { key, .. } if key == "peers[0]"),
        "{error:?}"
    );

    let mut stray = <&Map>::try_from(&expected().to_value().unwrap())
        .unwrap()
        .clone();
    stray.set("listne", "0.0.0.0:1");
    let schema = Daemon::schema().unwrap();
    assert!(
        validate_map(SchemaRef::new(&schema).unwrap(), &stray.into()).is_err(),
        "a misspelled key is refused by the schema"
    );
}
