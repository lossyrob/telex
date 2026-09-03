//! Thin runner: selects one consumer-shaped probe by argument or environment.
//!
//! This is deliberately not a CLI. It reads one selector (`argv[1]`, else
//! `TELEX_FIXTURE_PROBE`), runs the matching probe on the caller's own Tokio
//! runtime, prints the collected public evidence, and exits non-zero on any
//! failure. No Telex CLI parsing, no daemon serving, no sidecar.

use telex_application_client_consumer::{
    run_operator_station_probe, run_watcher_probe, ProbeConfig,
};

fn main() -> std::process::ExitCode {
    let probe = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("TELEX_FIXTURE_PROBE").ok())
        .unwrap_or_else(|| "watcher".to_string());

    let config = match ProbeConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("fixture-config-error: {error}");
            return std::process::ExitCode::from(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("fixture-runtime-error: {error}");
            return std::process::ExitCode::from(2);
        }
    };

    let result = runtime.block_on(async {
        match probe.as_str() {
            "watcher" => run_watcher_probe(&config).await,
            "station" | "operator-station" => run_operator_station_probe(&config).await,
            other => Err(format!("unknown probe '{other}' (expected watcher|station)")),
        }
    });

    match result {
        Ok(report) => {
            println!("probe={probe}");
            for line in report.lines {
                println!("{line}");
            }
            println!("probe-result=ok");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("probe={probe}");
            eprintln!("probe-error: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
