//! Benchmark for Alien's current Rust SDK (alien-platform-api). Same modes as
//! bench/rust; every mode prints one line, then waits for stdin EOF.

use std::{io::Read, time::Instant};

use alien_platform_api::Client;

const ID: &str = "mgr_0c29fq4a2yjb7kx3smwdgxlcmpqr";

fn done(line: &str) {
    println!("{line}");
    let mut sink = Vec::new();
    std::io::stdin().read_to_end(&mut sink).expect("read stdin");
}

fn arg(args: &[String], i: usize) -> usize {
    args[i].parse().expect("number")
}

async fn get(client: &Client) -> String {
    client.get_manager().id(ID).send().await.expect("get_manager").into_inner().id.to_string()
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let args = &argv[2..];
    match argv[1].as_str() {
        "import" => {
            Client::new("http://127.0.0.1:9");
            done("ready");
        }
        "get" => done(&get(&Client::new(&args[0])).await),
        "warm-get" => {
            let client = Client::new(&args[0]);
            let (count, n) = (arg(args, 1), arg(args, 2));
            for _ in 0..200 {
                get(&client).await;
            }
            let mut per_call = Vec::with_capacity(count);
            for _ in 0..count {
                let start = Instant::now();
                for _ in 0..n {
                    get(&client).await;
                }
                per_call.push(start.elapsed().as_nanos() as f64 / n as f64);
            }
            done(&serde_json::to_string(&per_call).expect("json"));
        }
        "page" => {
            let client = Client::new(&args[0]);
            let (size, warmup, n) = (arg(args, 1), arg(args, 2), arg(args, 3));
            let mut ms = Vec::with_capacity(n);
            for i in 0..warmup + n {
                let start = Instant::now();
                let page = client.list_managers().limit(std::num::NonZeroU64::new(size as u64).expect("size > 0")).send().await.expect("list_managers").into_inner();
                if i >= warmup {
                    ms.push(start.elapsed().as_nanos() as f64 / 1e6);
                }
                let cpu = page.last().and_then(|m| m.metrics.as_ref()).and_then(|m| m.cpu_usage_percent);
                assert!(page.len() == size && cpu == Some(12.25), "bad page");
            }
            done(&serde_json::to_string(&ms).expect("json"));
        }
        other => panic!("unknown mode {other}"),
    }
}
