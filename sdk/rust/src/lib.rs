//! Alien SDK for Rust: a thin facade over the generated BAML SDK (`baml_sdk`).
//!
//! It builds clients, renames, converts errors and picks the public surface;
//! every behavior lives in the BAML project (`baml/baml_src`).
//!
//! ```no_run
//! use alien_sdk::{Alien, Options, platform};
//!
//! # async fn run() -> Result<(), alien_sdk::Error> {
//! let alien = Alien::new(Options::default())?; // API key from ALIEN_API_KEY
//! let me = platform::auth::whoami_async(alien.platform()).await?;
//! # Ok(()) }
//! ```
//!
//! Every operation has an async function, `<name>_async`, and a blocking one,
//! `<name>`, each with a `_with_options` twin taking [`BamlOptions`] (a
//! per-call timeout, for example). The blocking ones fail with
//! [`Error::Runtime`] when called inside a Tokio runtime: use the `_async`
//! ones from async code. [`init`] and [`shutdown`] block.
//!
//! Telemetry is on (`medium`) unless `ALIEN_TELEMETRY=off`; see [`shutdown`]
//! for when to call it.

use std::{collections::HashMap, convert::Infallible, fmt, time::Duration};

use baml_sdk::setup;

/// Every Platform API operation and type, as generated: call an operation with
/// the client from [`Alien::platform`], for example
/// `platform::managers::get_async(alien.platform(), id).await?`.
/// `?` converts the generated error into [`Error`].
pub use baml_sdk::platform;

/// Every Manager API operation and type, as generated: call an operation with
/// the client from [`AlienManager::client`].
pub use baml_sdk::manager;

pub use baml_sdk::setup::CloudFormationLaunch;

/// Per-call options for the `*_with_options` functions, for example
/// `BamlOptions::new().timeout_ms(10_000)` (the default timeout is 5 minutes).
pub use baml_sdk::BamlOptions;

/// The types generated fields use: `json` fields are [`Json`]
/// (`serde_json::Value`), maps are [`Map`] (an insertion-ordered map).
pub use baml_bridge::{Json, Map, serde_json};

/// The Platform API used when [`Options::server_url`] is `None`.
pub const DEFAULT_SERVER_URL: &str = "https://api.alien.dev";

/// Read by [`Alien::new`] when [`Options::api_key`] is `None`, as Alien's
/// TypeScript SDK and CLI do.
const API_KEY_ENV: &str = "ALIEN_API_KEY";

/// What `Debug` output shows instead of a secret.
const REDACTED: &str = "<redacted>";

/// `url` with its query string, which can carry a credential, redacted.
fn redact_query(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?{REDACTED}"),
        None => url.to_string(),
    }
}

/// The error every Alien SDK call returns for a failure the SDK describes
/// (docs/errors.md). Its fields are named as in `alien_error::AlienError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlienError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    /// The HTTP status of the response that failed, if there was one.
    pub http_status_code: Option<u16>,
    pub context: Option<HashMap<String, String>>,
}

impl fmt::Display for AlienError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AlienError {}

impl From<baml_sdk::errors::AlienError> for AlienError {
    fn from(value: baml_sdk::errors::AlienError) -> Self {
        Self {
            code: value.code,
            message: value.message,
            retryable: value.retryable,
            // The BAML code sets it only from an HTTP response's status.
            http_status_code: value
                .http_status
                .map(|status| u16::try_from(status).expect("an HTTP status code fits in u16")),
            context: value.context.map(|context| context.into_iter().collect()),
        }
    }
}

/// An SDK call failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The call failed in a way the SDK describes (docs/errors.md).
    Alien(AlienError),
    /// The BAML runtime failed: it could not start or was shut down, a
    /// blocking function was called inside an async runtime, a value has no
    /// Rust form (a `json` integer outside `i64`/`u64` in a response, docs/gaps.md#b19),
    /// or BAML panicked. Holds the `baml_bridge` error, BAML trace included;
    /// `Display` and `source()` are the held error's.
    Runtime(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Alien(error) => error.fmt(f),
            Error::Runtime(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Alien(_) => None,
            Error::Runtime(error) => error.source(),
        }
    }
}

impl From<baml_bridge::Error<baml_sdk::errors::AlienError>> for Error {
    fn from(error: baml_bridge::Error<baml_sdk::errors::AlienError>) -> Self {
        match error {
            baml_bridge::Error::Thrown { value, .. } => Error::Alien((*value).into()),
            other => Error::Runtime(Box::new(other)),
        }
    }
}

impl From<baml_bridge::Error> for Error {
    fn from(error: baml_bridge::Error) -> Self {
        Error::Runtime(Box::new(error))
    }
}

/// Starts the BAML runtime now rather than on the first call, so that a
/// failure to start shows here. Blocks. After [`shutdown`] it returns `Ok` but
/// does not restart the runtime.
pub fn init() -> Result<(), Error> {
    Ok(baml_sdk::init()?)
}

/// Shuts the SDK's runtime down: waits for calls in flight and seals the
/// telemetry recording. SDK calls made afterwards fail with
/// [`Error::Runtime`] for the rest of the process ([`init`] does not restart
/// it). Calling it again returns `Ok`.
///
/// It blocks until every call in flight has finished, with no time limit
/// (each call can run up to its timeout, 5 minutes by default). From async
/// code, run it with `tokio::task::spawn_blocking`.
///
/// The runtime does this by itself when the process exits normally (`main`
/// returns, or `std::process::exit` on Unix), with the same wait and no time limit.
/// Unless telemetry is off (`ALIEN_TELEMETRY=off`), call `shutdown()` before
/// the process ends any other way: a signal (handle SIGTERM, for example in a
/// container with a 30-second grace period), `std::process::abort`,
/// `panic = "abort"`, or `std::process::exit` on Windows. Otherwise the
/// recording is not sealed (docs/runtime.md).
pub fn shutdown() -> Result<(), Error> {
    baml_bridge::runtime::shutdown()
        .map_err(|error| Error::from(baml_bridge::Error::<Infallible>::Sdk(error)))
}

/// How to reach the Platform API: Alien's TypeScript SDK options (`apiKey`,
/// `serverURL`, `workspace`, `timeoutMs`) in snake_case. Its `Debug` output
/// never shows the API key.
#[derive(Clone, Default)]
pub struct Options {
    /// Sent as a Bearer token. When `None`, [`Alien::new`] reads it from the
    /// `ALIEN_API_KEY` environment variable, as Alien's TypeScript SDK does.
    /// An empty key, or none at all, makes [`Alien::new`] fail.
    pub api_key: Option<String>,
    /// Defaults to [`DEFAULT_SERVER_URL`].
    pub server_url: Option<String>,
    /// Optional for workspace-scoped API keys.
    pub workspace: Option<String>,
    /// Limit for each HTTP request, from connecting to reading the whole
    /// response, rounded up to whole milliseconds. `None`: BAML's default of 5
    /// minutes; `Some(Duration::ZERO)`: no limit. A request that runs out fails
    /// with `COMMUNICATION_ERROR` (retryable). A per-call timeout
    /// ([`BamlOptions::timeout_ms`]) limits the whole call as well: the shorter
    /// one ends it.
    pub timeout: Option<Duration>,
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("api_key", &self.api_key.as_ref().map(|_| REDACTED))
            .field("server_url", &self.server_url.as_deref().map(redact_query))
            .field("workspace", &self.workspace)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// The largest BAML `int`: the generated clients hold the timeout as one.
const MAX_TIMEOUT_MS: u128 = (1 << 62) - 1;

/// A client timeout as the generated clients take it: whole milliseconds,
/// rounded up so that a sub-millisecond timeout still limits.
fn timeout_ms(timeout: Option<Duration>) -> Option<i64> {
    timeout.map(|t| {
        i64::try_from(t.as_nanos().div_ceil(1_000_000).min(MAX_TIMEOUT_MS))
            .expect("at most 2^62 - 1")
    })
}

/// A Platform API client. Its `Debug` output never shows the API key (the
/// generated client that [`Alien::platform`] returns does).
#[derive(Clone)]
pub struct Alien {
    client: platform::PlatformClient,
}

impl fmt::Debug for Alien {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Alien")
            .field("server_url", &redact_query(&self.client.base_url))
            .field("api_key", &REDACTED)
            .field("workspace", &self.client.workspace)
            .field("timeout_ms", &self.client.timeout_ms)
            .finish()
    }
}

impl Alien {
    /// Creates a client. Fails with `INVALID_REQUEST` when there is no API
    /// key: [`Options::api_key`] is empty, or `None` with `ALIEN_API_KEY`
    /// unset or empty. Call [`shutdown`] before the process ends other than by
    /// exiting normally (a signal, an abort).
    pub fn new(options: Options) -> Result<Self, Error> {
        let api_key = options
            .api_key
            .unwrap_or_else(|| std::env::var(API_KEY_ENV).unwrap_or_default());
        if api_key.is_empty() {
            return Err(Error::Alien(AlienError {
                code: "INVALID_REQUEST".to_string(),
                message: format!("no API key: pass api_key or set {API_KEY_ENV}"),
                retryable: false,
                http_status_code: None,
                context: None,
            }));
        }
        Ok(Self {
            client: platform::PlatformClient {
                base_url: options
                    .server_url
                    .unwrap_or_else(|| DEFAULT_SERVER_URL.to_string()),
                api_key,
                workspace: options.workspace,
                timeout_ms: timeout_ms(options.timeout),
            },
        })
    }

    /// The client every operation in [`platform`] takes first (generated
    /// operations take it by reference or by value). Its `Debug` output shows
    /// the API key.
    pub fn platform(&self) -> &platform::PlatformClient {
        &self.client
    }

    /// Setup helpers that need the Platform API.
    pub fn setup(&self) -> ProjectSetup<'_> {
        ProjectSetup {
            client: &self.client,
        }
    }
}

/// A Manager API client, for example for the `managerUrl` and `token` that
/// `platform::commands::bootstrap_async` returns. Its `Debug` output never
/// shows the token (the generated client that [`AlienManager::client`]
/// returns does).
#[derive(Clone)]
pub struct AlienManager {
    client: manager::ManagerClient,
}

impl fmt::Debug for AlienManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AlienManager")
            .field("server_url", &redact_query(&self.client.base_url))
            .field("token", &self.client.token.as_ref().map(|_| REDACTED))
            .field("timeout_ms", &self.client.timeout_ms)
            .finish()
    }
}

impl AlienManager {
    /// Creates a client. The token is sent as a Bearer token on every
    /// operation when set; an empty token sends none. Call [`shutdown`] before
    /// the process ends other than by exiting normally (a signal, an abort).
    pub fn new(server_url: impl Into<String>, token: Option<String>) -> Self {
        let token = token.filter(|token| !token.is_empty());
        Self {
            client: manager::ManagerClient {
                base_url: server_url.into(),
                token,
                timeout_ms: None,
            },
        }
    }

    /// The client with a limit for each HTTP request, as [`Options::timeout`]
    /// sets it for [`Alien`] (`Duration::ZERO`: no limit). Without it, BAML's
    /// default of 5 minutes applies:
    /// `AlienManager::new(url, token).with_timeout(Duration::from_secs(30))`.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.client.timeout_ms = timeout_ms(Some(timeout));
        self
    }

    /// The client every operation in [`manager`] takes first (generated
    /// operations take it by reference or by value). Its `Debug` output shows
    /// the token.
    pub fn client(&self) -> &manager::ManagerClient {
        &self.client
    }
}

/// Setup helpers that need the Platform API.
pub struct ProjectSetup<'a> {
    client: &'a platform::PlatformClient,
}

impl ProjectSetup<'_> {
    /// Fetches the project's AWS template and builds a quick-create link for it.
    pub async fn launch_url_for_project(
        &self,
        project: &str,
        region: &str,
        stack_name: &str,
        parameters: impl IntoIterator<Item = (String, String)>,
    ) -> Result<String, Error> {
        Ok(setup::launch_url_for_project_async(
            self.client,
            project.to_string(),
            region.to_string(),
            stack_name.to_string(),
            parameters.into_iter().collect(),
        )
        .await?)
    }
}

/// Builds an AWS CloudFormation quick-create link from a template URL.
pub async fn cloudformation_launch_url(input: CloudFormationLaunch) -> Result<String, Error> {
    Ok(setup::cloudformation_launch_url_async(input).await?)
}
