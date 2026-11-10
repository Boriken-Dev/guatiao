// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Types that derive both libraries' macros, so one declaration is described twice and the
//! two descriptions can be compared. Nothing outside these tests derives both.

#![allow(dead_code, unused)]

extern crate alloc;

use alloc::collections::BTreeMap;
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;
use std::path::PathBuf;

use coqui_builder::Describe;
use guatiao::{FromValue, Schema, ToValue, Value};
use guatiao_serde::Presentation;

/// `MAJOR.MINOR`: a text form and no value traits of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl core::str::FromStr for Version {
    type Err = String;

    fn from_str(text: &str) -> Result<Version, String> {
        let (major, minor) = text.split_once('.').ok_or("a version is MAJOR.MINOR")?;
        Ok(Version {
            major: major.parse().map_err(|_| "a version is MAJOR.MINOR")?,
            minor: minor.parse().map_err(|_| "a version is MAJOR.MINOR")?,
        })
    }
}

// A unit enum with labels and a renamed name.
#[derive(Describe, ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq)]
pub enum Level {
    /// Nothing at all.
    Off,
    #[map(rename = "warn")]
    #[schema(label = "Warnings only")]
    Warning,
    On,
}

/// A unit enum that keeps a name it does not know.
#[derive(Describe, ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
pub enum Codec {
    H264,
    #[schema(label = "AV1 video")]
    Av1,
    #[map(other)]
    Other(String),
}

#[derive(Describe, ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(transparent)]
pub struct Name(pub String);

#[derive(Describe, ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
pub struct Waiting {
    /// How many at once.
    #[map(default = 32)]
    #[schema(title = "Total")]
    pub total: u32,
    #[map(default = Some(8u32), none = "unbounded")]
    pub per_source: Option<u32>,
}

#[derive(Describe, ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
pub struct Common {
    #[map(default)]
    pub verbose: bool,
    #[map(default = 3)]
    pub retries: u8,
}

#[derive(Describe, ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(tag = "kind")]
pub enum Credential {
    /// A user and a password.
    #[schema(title = "Password")]
    Password {
        user: String,
        #[schema(sensitive)]
        password: String,
    },
    KeyFile {
        path: PathBuf,
    },
    Anonymous,
}

/// Every shape the two derives share.
#[derive(Describe, ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
pub struct Everything {
    /// Where to connect.
    pub host: String,
    #[map(default = 8080)]
    pub port: u16,
    pub nickname: Option<String>,
    #[map(rename = "view-only", default)]
    pub view_only: bool,
    #[map(skip)]
    pub scratch: Vec<String>,
    pub waiting: Waiting,
    #[map(flatten)]
    pub common: Common,
    #[schema(min_items = 1, max_items = 4)]
    pub ports: Vec<u16>,
    pub env: BTreeMap<String, String>,
    #[map(default = Level::Warning)]
    pub level: Level,
    pub codec: Codec,
    pub auth: Credential,
    pub alias: Name,
    pub address: Ipv4Addr,
    pub bind: SocketAddr,
    pub root: PathBuf,
    pub initial: char,
    pub offset: i64,
    #[schema(multiple_of = 0.25)]
    pub ratio: f64,
    #[schema(
        title = "Contact",
        section = "People",
        order = 3,
        advanced,
        read_only,
        deprecated,
        format = "email",
        pattern = "^.+@.+$",
        min_length = 3,
        max_length = 40,
        examples("a@b.c".to_string())
    )]
    pub email: String,
    #[schema(sensitive)]
    pub token: String,
    #[map(default = Some(5u16), none = "off")]
    pub cap: Option<u16>,
    #[map(default, as_text)]
    pub peers: Vec<Version>,
    #[map(as_text)]
    pub fallbacks: Option<Vec<Version>>,
    #[map(as_text, default = Version { major: 1, minor: 0 })]
    pub floor: Version,
}

pub fn everything() -> Everything {
    Everything {
        host: "example.org".into(),
        port: 9000,
        nickname: Some("nick".into()),
        view_only: true,
        scratch: Vec::new(),
        waiting: Waiting {
            total: 10,
            per_source: None,
        },
        common: Common {
            verbose: true,
            retries: 7,
        },
        ports: vec![80, 443],
        env: BTreeMap::from([("A".into(), "1".into()), ("B".into(), "2".into())]),
        level: Level::On,
        codec: Codec::Other("vp9".into()),
        auth: Credential::Password {
            user: "root".into(),
            password: "hunter2".into(),
        },
        alias: Name("al".into()),
        address: Ipv4Addr::new(10, 0, 0, 1),
        bind: "127.0.0.1:80".parse().unwrap(),
        root: PathBuf::from("/srv"),
        initial: 'x',
        offset: -4,
        ratio: 0.75,
        email: "a@b.c".into(),
        token: "t0k3n".into(),
        cap: None,
        peers: vec![
            Version { major: 1, minor: 2 },
            Version { major: 3, minor: 4 },
        ],
        fallbacks: Some(vec![Version { major: 9, minor: 9 }]),
        floor: Version { major: 2, minor: 7 },
    }
}

/// A compact, order-preserving JSON text of a value, for messages.
pub fn json(value: &Value) -> String {
    guatiao_serde::text::json::to_string_pretty(value, Presentation::new())
        .unwrap_or_else(|e| format!("<{e}>"))
}

/// A JSON text as a value.
pub fn parse(text: &str) -> Value {
    guatiao_serde::text::json::from_str(text, guatiao::Alloc::rust(), Presentation::new())
        .unwrap_or_else(|e| panic!("not JSON: {e}"))
}

/// The places two values differ, as `path: left | right`, in key order.
pub fn diff(left: &Value, right: &Value) -> Vec<String> {
    let (left, right): (serde_json::Value, serde_json::Value) = (
        serde_json::from_str(&json(left)).expect("JSON"),
        serde_json::from_str(&json(right)).expect("JSON"),
    );
    let mut out = Vec::new();
    walk(&left, &right, String::new(), &mut out);
    out
}

fn walk(left: &serde_json::Value, right: &serde_json::Value, at: String, out: &mut Vec<String>) {
    use serde_json::Value as J;
    match (left, right) {
        (J::Object(a), J::Object(b)) => {
            let keys: alloc::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let here = format!("{at}.{key}");
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => walk(x, y, here, out),
                    (Some(x), None) => out.push(format!("{here}: {x} | (absent)")),
                    (None, Some(y)) => out.push(format!("{here}: (absent) | {y}")),
                    (None, None) => {}
                }
            }
        }
        (J::Array(a), J::Array(b)) if a.len() == b.len() => {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                walk(x, y, format!("{at}[{i}]"), out);
            }
        }
        (a, b) if a == b => {}
        (a, b) => out.push(format!("{at}: {a} | {b}")),
    }
}
