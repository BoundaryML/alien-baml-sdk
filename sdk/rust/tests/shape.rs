//! Compile-time checks that the Rust calls docs/api-shape.md and the README
//! show type-check: one required argument, optional arguments as setters, no
//! arguments, no response body, a Manager operation, per-call options, the
//! setup helper and `shutdown`, with `send()` returning `alien_sdk::Error`, a
//! request body built from the generated `new` with struct update syntax, and
//! a match on a union's catch-all. Nothing here runs.

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
    let _m: platform::Manager = alien.managers().get(&id).send().await?;
    let _page: platform::ListDeploymentsResponse = alien
        .deployments()
        .list()
        .project("my-project")
        .limit(10)
        .send()
        .await?;
    let _next = _page.next_cursor;
    let _t = alien.deployment_groups().create_token(&id).send().await?;
    let _me = alien.auth().whoami().send().await?;
    alien.api_keys().revoke(key_id).send().await?;
    let _invitation = alien.accept_workspace_invitation("token").send().await?;
    let mgr = AlienManager::new(manager_url, Some(token));
    let _cmd: manager::CreateCommandResponse = mgr.commands().create_command(req).send().await?;

    let _m = alien
        .managers()
        .get(id)
        .baml_options(BamlOptions::new().timeout_ms(10_000))
        .send()
        .await?;
    let _url: String = alien
        .setup()
        .launch_url_for_project("my-project", "us-east-1", "alien-setup", [])
        .await?;
    let _key_from_env = Alien::new(Options::default())?;

    // A request body: the required fields go to `new`, optional ones are set
    // over it, so the code keeps compiling when the API adds an optional field.
    let body = platform::CreateSetupLinkRequest {
        max_deployments: Some(5),
        ..platform::CreateSetupLinkRequest::new("cust-1".into(), "Customer".into(), "web".into())
    };
    let _link: platform::CreateSetupLinkResponse = alien.setup_links().create(body).send().await?;

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
