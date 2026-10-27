//! Compile-time checks that the Rust calls docs/api-shape.md and the README
//! show type-check: required arguments, positional optional ones (`None` or a
//! value), no arguments, no response body, a Manager operation, per-call
//! options, the setup helper and `shutdown`, with `?` converting the generated
//! error into `alien_sdk::Error`, and a match on a union's catch-all. Nothing
//! here runs.

use alien_sdk::{Alien, AlienManager, BamlOptions, Error, Options, manager, platform};

#[allow(dead_code)]
async fn documented_calls(
    id: String,
    key_id: String,
    manager_url: String,
    token: String,
    req: manager::CreateCommandRequest,
) -> Result<(), Error> {
    let alien = Alien::new(Options {
        api_key: Some("key".into()),
        workspace: Some("my-workspace".into()),
        ..Default::default()
    })?;
    let _m: platform::Manager = platform::managers::get_async(alien.platform(), id.clone()).await?;
    #[rustfmt::skip] // the eight `None`s on one line, as in docs/api-shape.md
    let _page: platform::ListDeploymentsResponse = platform::deployments::list_async(
        alien.platform(),
        Some("my-project".to_string()), // project
        None, None, None, None, None, None, None, None, // deploymentGroup … include
        Some(10),                       // limit
        None,                           // cursor
    )
    .await?;
    let _t =
        platform::deploymentGroups::createToken_async(alien.platform(), id.clone(), None).await?;
    let _me = platform::auth::whoami_async(alien.platform()).await?;
    platform::apiKeys::revoke_async(alien.platform(), key_id).await?;
    let mgr = AlienManager::new(manager_url, Some(token));
    let _cmd: manager::CreateCommandResponse =
        manager::commands::createCommand_async(mgr.client(), req).await?;

    let _m = platform::managers::get_async_with_options(
        alien.platform(),
        id,
        BamlOptions::new().timeout_ms(10_000),
    )
    .await?;
    let _url: String = alien
        .setup()
        .launch_url_for_project("my-project", "us-east-1", "alien-setup", [])
        .await?;
    let _key_from_env = Alien::new(Options::default())?;
    alien_sdk::shutdown()?;
    Ok(())
}

/// A union tagged by two fields has one catch-all, matched as in
/// docs/api-shape.md ("Unions"): here a `storage` binding with a `service`
/// this SDK does not know.
#[allow(dead_code)]
fn binding_service(b: &platform::ExternalBinding) -> Option<String> {
    match b {
        platform::ExternalBinding::ExternalBindingOther(o) => Some(o.raw["service"].to_string()),
        _ => None,
    }
}
