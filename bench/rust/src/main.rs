//! Benchmarks for the Rust SDK (bench/run.py drives it and lists the modes).
//! Every mode prints one line, then waits for stdin EOF so the driver can read RSS.

use std::{future::Future, io::Read, time::Instant};

use alien_sdk::{Alien, CloudFormationLaunch, Options, platform};

const ID: &str = "mgr_0c29fq4a2yjb7kx3smwdgxlcmpqr";

fn input() -> CloudFormationLaunch {
    CloudFormationLaunch {
        templateUrl: "https://example.com/setup.yaml".into(),
        region: "us-east-1".into(),
        stackName: "alien-setup".into(),
        parameters: [("ExternalId".to_string(), "customer-0001".to_string())].into_iter().collect(),
    }
}

fn client(server_url: &str) -> Result<Alien, alien_sdk::Error> {
    Alien::new(Options { api_key: Some("alien_bench_key".into()), server_url: Some(server_url.into()), workspace: None, timeout: None })
}

fn done(line: &str) {
    println!("{line}");
    let mut sink = Vec::new();
    std::io::stdin().read_to_end(&mut sink).expect("read stdin");
}

/// Per-call nanoseconds of `call`, one value per batch, after `warmup` calls.
async fn batches<F, Fut, T, E>(call: F, count: usize, n: usize, warmup: usize) -> Vec<f64>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    for _ in 0..warmup {
        call().await.expect("call");
    }
    let mut per_call = Vec::with_capacity(count);
    for _ in 0..count {
        let start = Instant::now();
        for _ in 0..n {
            call().await.expect("call");
        }
        per_call.push(start.elapsed().as_nanos() as f64 / n as f64);
    }
    per_call
}

fn arg(args: &[String], i: usize) -> usize {
    args[i].parse().expect("number")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), alien_sdk::Error> {
    let argv: Vec<String> = std::env::args().collect();
    let args = &argv[2..];
    match argv[1].as_str() {
        "import" => {
            alien_sdk::init()?;
            client("http://127.0.0.1:9")?;
            done("ready");
        }
        "get" => {
            let alien = client(&args[0])?;
            done(&platform::managers::get_async(alien.platform(), ID.into()).await?.id);
        }
        "warm-get" => {
            let alien = client(&args[0])?;
            let per_call = batches(|| platform::managers::get_async(alien.platform(), ID.into()), arg(args, 1), arg(args, 2), 200).await;
            done(&serde_json::to_string(&per_call).expect("json"));
        }
        "page" => {
            let alien = client(&args[0])?;
            let (size, warmup, n) = (arg(args, 1), arg(args, 2), arg(args, 3));
            let mut ms = Vec::with_capacity(n);
            for i in 0..warmup + n {
                let start = Instant::now();
                let page = platform::managers::list_async(alien.platform(), None, Some(size as i64)).await?;
                if i >= warmup {
                    ms.push(start.elapsed().as_nanos() as f64 / 1e6);
                }
                let cpu = page.last().and_then(|m| m.metrics.as_ref()).and_then(|m| m.cpuUsagePercent);
                assert!(page.len() == size && cpu == Some(12.25), "bad page");
            }
            done(&serde_json::to_string(&ms).expect("json"));
        }
        "warm-helper" => {
            let per_call = batches(|| alien_sdk::cloudformation_launch_url(input()), arg(args, 0), arg(args, 1), 1000).await;
            done(&serde_json::to_string(&per_call).expect("json"));
        }
        "warm-noop" => {
            alien_sdk::init()?;
            let per_call = batches(baml_sdk::compat::newer_remote_sandbox_async, arg(args, 0), arg(args, 1), 1000).await;
            done(&serde_json::to_string(&per_call).expect("json"));
        }
        other => panic!("unknown mode {other}"),
    }
    Ok(())
}
