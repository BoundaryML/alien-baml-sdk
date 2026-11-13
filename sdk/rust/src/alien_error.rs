//! The SDK's errors as `alien_error::AlienError`, Alien's own error type
//! (cargo feature `alien-error`, on by default; docs/errors.md).
//!
//! ```no_run
//! use alien_sdk::alien_error::{self, AlienErrorData, Context};
//! use alien_sdk::{Alien, SdkResultExt};
//! # #[derive(Debug, Clone, serde::Serialize)]
//! # struct ManagerLookup;
//! # impl AlienErrorData for ManagerLookup {
//! #     fn code(&self) -> &'static str { "MANAGER_LOOKUP_FAILED" }
//! #     fn retryable(&self) -> bool { false }
//! #     fn internal(&self) -> bool { false }
//! #     fn message(&self) -> String { "cannot look the manager up".into() }
//! # }
//!
//! // `?` converts where the function returns alien_error's Result:
//! async fn name(alien: &Alien, id: &str) -> alien_error::Result<String> {
//!     Ok(alien.managers().get(id).send().await?.name)
//! }
//!
//! // `into_sdk_error()` converts in place, for `.context(..)` to chain onto:
//! async fn checked(alien: &Alien, id: &str) -> alien_error::Result<String, ManagerLookup> {
//!     let manager = alien.managers().get(id).send().await.into_sdk_error().context(ManagerLookup)?;
//!     Ok(manager.name)
//! }
//! ```
//!
//! Use `?` or [`SdkResultExt::into_sdk_error`], not `alien_error`'s
//! `.into_alien_error()`. That method exists for every `std::error::Error`,
//! this SDK's included, and reads only what such an error has: it turns the
//! error and each of its sources into a `GENERIC_ERROR` whose message is
//! `"<code>: <message>"`, and loses the code, the retryable flag, the HTTP
//! status, the hint and the context.

use alien_error::{AlienError as Converted, GenericError};

use crate::serde_json::{Value, json};
use crate::{AlienError, Error};

/// A context with the request ID in it, where `alien_error::AlienError`,
/// which has no field for one, keeps it and Alien's tools look for it: the
/// rule of Alien's own Platform client (`context_with_request_id`). An object
/// gains `requestId`, unless it has that key already; no context becomes
/// `{"requestId": ..}`; any other value becomes `{"requestId": ..,
/// "context": <the value>}`. Without a request ID the context is unchanged.
fn context_with_request_id(context: Option<Value>, request_id: Option<String>) -> Option<Value> {
    let Some(request_id) = request_id else {
        return context;
    };
    match context {
        Some(Value::Object(mut object)) => {
            object
                .entry("requestId")
                .or_insert_with(|| Value::String(request_id));
            Some(Value::Object(object))
        }
        Some(value) => Some(json!({ "requestId": request_id, "context": value })),
        None => Some(json!({ "requestId": request_id })),
    }
}

impl From<AlienError> for Converted {
    /// Every field of the error, and of each level of its `source` chain:
    ///
    /// | `alien_error::AlienError` | From |
    /// |---|---|
    /// | `code`, `message`, `retryable`, `hint` | The same |
    /// | `http_status_code` | The same, `None` included |
    /// | `internal` | The same; `false` when the SDK error has none |
    /// | `context` | `context`, with `request_id` merged in (above) |
    /// | `source` | `source`, converted the same way |
    /// | `error` | `Some(GenericError { message })` |
    /// | `human_layer_presentation` | The default, `Normal` |
    fn from(error: AlienError) -> Self {
        // Every field by name, without `..`: a field the SDK's error gains
        // does not compile until it is mapped here.
        let AlienError {
            code,
            message,
            retryable,
            http_status_code,
            context,
            hint,
            request_id,
            source,
            internal,
        } = error;
        // Built by the constructor and assigned field by field, not as a
        // struct literal: `alien_error::AlienError` is not `#[non_exhaustive]`,
        // so a literal would stop compiling for everyone when it gains a field.
        let mut converted = Converted::new(GenericError {
            message: message.clone(),
        });
        converted.code = code;
        converted.message = message;
        converted.retryable = retryable;
        converted.internal = internal.unwrap_or(false);
        // The constructor fills in GenericError's 500; this is the response's
        // status, or none.
        converted.http_status_code = http_status_code;
        converted.hint = hint;
        converted.context = context_with_request_id(context, request_id);
        converted.source = source.map(|source| Box::new(Self::from(*source)));
        converted
    }
}

impl From<Error> for Converted {
    /// [`Error::Alien`] keeps every field (`From<alien_sdk::AlienError>`).
    /// [`Error::Runtime`], a failure of the BAML runtime itself (a per-call
    /// timeout or a cancellation among them), has no code of Alien's: it
    /// converts as `alien_error`'s `.into_alien_error()` converts any
    /// `std::error::Error`, to a `GENERIC_ERROR` with the error's text as its
    /// message, not retryable, and its sources below it.
    fn from(error: Error) -> Self {
        match error {
            Error::Alien(error) => error.into(),
            Error::Runtime(error) => Converted::from_std(&*error),
        }
    }
}

/// Converts the error of an SDK call's result to `alien_error::AlienError`,
/// for code that handles errors as Alien's does. It has the name and the
/// meaning of the `SdkResultExt` of Alien's own Rust clients:
///
/// ```ignore
/// let manager = alien.managers().get(id).send().await
///     .into_sdk_error()          // Result<_, alien_error::AlienError>
///     .context(MyError::Lookup)?; // chains as Alien's code does
/// ```
///
/// `?` does the same conversion in a function that returns
/// `alien_error::Result<T>`. To import this trait next to another
/// `SdkResultExt`, import it unnamed: `use alien_sdk::SdkResultExt as _;`.
pub trait SdkResultExt<T> {
    /// The result with its error converted, every field kept
    /// (`From<alien_sdk::Error> for alien_error::AlienError`).
    fn into_sdk_error(self) -> Result<T, Converted>;
}

impl<T> SdkResultExt<T> for Result<T, Error> {
    fn into_sdk_error(self) -> Result<T, Converted> {
        self.map_err(Converted::from)
    }
}

impl<T> SdkResultExt<T> for Result<T, AlienError> {
    fn into_sdk_error(self) -> Result<T, Converted> {
        self.map_err(Converted::from)
    }
}
