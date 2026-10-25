// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a script's failure crosses as: a `GuatiaoError` is its own status,
//! QuickJS's out-of-memory and stack errors are the limits, anything else
//! is `GUATIAO_ERR_INTERNAL` in its words.

use guatiao::Status;
use guatiao::library::ProviderError;
use guatiao_bridge::{BridgeError, Limit};
use rquickjs::{Coerced, Ctx, Error, Value};

use crate::prelude::Core;
use crate::values::Conversion;

/// The limits whose words an error can take.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Words {
    pub(crate) memory: Option<u64>,
    pub(crate) depth: Option<u64>,
}

pub(crate) fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// A refusal in words, for a value that cannot cross.
pub(crate) fn bad_value(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

/// What a value thrown by a script came to.
pub(crate) fn thrown<'js>(
    core: Option<&Core<'js>>,
    value: &Value<'js>,
    words: Words,
) -> ProviderError {
    let Some(object) = value.as_object() else {
        let text = value
            .get::<Coerced<String>>()
            .map_or_else(|_| value.type_name().to_string(), |c| c.0);
        return ProviderError::new(
            Status::GUATIAO_ERR_INTERNAL,
            first_line(&format!("thrown: {text}")),
        );
    };
    let text = |key: &str| {
        object
            .get::<_, Option<Coerced<String>>>(key)
            .ok()
            .flatten()
            .map(|c| c.0)
            .unwrap_or_default()
    };
    let message = text("message");
    if let Some(core) = core
        && object.is_instance_of(&core.guatiao_error)
        && let Ok(status) = object.get::<_, u32>("status")
    {
        return ProviderError::new(Status::from(status), &message);
    }
    let name = text("name");
    if name == "InternalError" && message == "out of memory" {
        return match words.memory {
            Some(at) => BridgeError::Limit {
                limit: Limit::Memory,
                at,
            }
            .into(),
            None => ProviderError::new(Status::GUATIAO_ERR_ALLOC, "out of memory"),
        };
    }
    if name == "RangeError"
        && message.contains("call stack")
        && let Some(at) = words.depth
    {
        return BridgeError::Limit {
            limit: Limit::Depth,
            at,
        }
        .into();
    }
    let said = if name.is_empty() {
        message
    } else {
        format!("{name}: {message}")
    };
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, first_line(&said))
}

/// What a failed QuickJS call came to; the exception, when there is one,
/// is taken from the context.
pub(crate) fn caught<'js>(
    ctx: &Ctx<'js>,
    core: Option<&Core<'js>>,
    error: &Error,
    words: Words,
) -> ProviderError {
    match error {
        Error::Exception => thrown(core, &ctx.catch(), words),
        Error::Allocation => match words.memory {
            Some(at) => BridgeError::Limit {
                limit: Limit::Memory,
                at,
            }
            .into(),
            None => ProviderError::new(Status::GUATIAO_ERR_ALLOC, "out of memory"),
        },
        other => ProviderError::new(Status::GUATIAO_ERR_INTERNAL, first_line(&other.to_string())),
    }
}

/// A value that could not cross, as a refusal.
pub(crate) fn conversion<'js>(
    ctx: &Ctx<'js>,
    core: Option<&Core<'js>>,
    error: &Conversion,
    words: Words,
) -> ProviderError {
    match error {
        Conversion::Js(e) => caught(ctx, core, e, words),
        Conversion::Refused(why) => bad_value(why),
    }
}
