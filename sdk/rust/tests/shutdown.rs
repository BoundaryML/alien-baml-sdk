//! `shutdown()`: its own test binary with one test, because SDK calls fail for
//! the rest of the process once the runtime is shut down.

use alien_sdk::{CloudFormationLaunch, Error};

fn input() -> CloudFormationLaunch {
    CloudFormationLaunch {
        templateUrl: "https://example.com/setup.yaml".into(),
        region: "us-east-1".into(),
        stackName: "alien-setup".into(),
        parameters: Default::default(),
    }
}

async fn shutdown() -> Result<(), Error> {
    tokio::task::spawn_blocking(alien_sdk::shutdown)
        .await
        .expect("shutdown task")
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_fail_after_shutdown() {
    alien_sdk::cloudformation_launch_url(input())
        .await
        .expect("a call before shutdown");
    shutdown().await.expect("shutdown");
    shutdown().await.expect("a second shutdown");

    let error = alien_sdk::cloudformation_launch_url(input())
        .await
        .expect_err("a call after shutdown");
    assert!(
        matches!(error, Error::Runtime(_)),
        "expected Error::Runtime, got {error:?}"
    );

    alien_sdk::init().expect("init after shutdown");
    let error = alien_sdk::cloudformation_launch_url(input())
        .await
        .expect_err("a call after init after shutdown");
    assert!(
        matches!(error, Error::Runtime(_)),
        "expected Error::Runtime, got {error:?}"
    );
}
