//! Ephemeral delivery client: blocks on the holder, exits with the message.
//!
//! Exit codes are the client contract:
//!   0 = message delivered (printed as JSON on stdout)
//!   2 = idle timeout (no message within --timeout-ms)
//!   3 = holder gone (connect/read failed or EOF)
//!   4 = holder hung (no frame within --hang-ms, or DB heartbeat gone stale)

use clap::Parser;
use std::time::{Duration, Instant};
use telex_spike::{Frame, Request};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    address: String,
    #[arg(long, default_value_t = 47655)]
    port: u16,
    #[arg(long, default_value_t = 30000)]
    timeout_ms: u64,
    /// If no frame arrives within this window, treat the holder as hung.
    #[arg(long, default_value_t = 8000)]
    hang_ms: u64,
    /// DB heartbeat age beyond which the holder is considered degraded/hung.
    #[arg(long, default_value_t = 15000)]
    stale_heartbeat_ms: i64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    std::process::exit(run(args).await);
}

async fn run(args: Args) -> i32 {
    let stream = match TcpStream::connect(("127.0.0.1", args.port)).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[waiter] holder-gone (connect failed): {e}");
            return 3;
        }
    };
    let (read_half, mut write_half) = stream.into_split();

    let req = Request::Wait {
        address: args.address.clone(),
        since: 0,
        timeout_ms: args.timeout_ms,
    };
    let mut line = serde_json::to_string(&req).unwrap();
    line.push('\n');
    if write_half.write_all(line.as_bytes()).await.is_err() {
        eprintln!("[waiter] holder-gone (write failed)");
        return 3;
    }
    let _ = write_half.flush().await;

    let mut reader = BufReader::new(read_half);
    let hang = Duration::from_millis(args.hang_ms);
    let overall_deadline = Instant::now() + Duration::from_millis(args.timeout_ms + 5000);

    loop {
        let mut buf = String::new();
        match tokio::time::timeout(hang, reader.read_line(&mut buf)).await {
            Err(_) => {
                eprintln!("[waiter] HUNG: no frame within {} ms", args.hang_ms);
                return 4;
            }
            Ok(Ok(0)) => {
                eprintln!("[waiter] holder-gone (EOF)");
                return 3;
            }
            Ok(Err(e)) => {
                eprintln!("[waiter] holder-gone (read error): {e}");
                return 3;
            }
            Ok(Ok(_)) => {
                let frame: Frame = match serde_json::from_str(buf.trim()) {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("[waiter] bad frame: {e} ({})", buf.trim());
                        continue;
                    }
                };
                match frame {
                    Frame::Keepalive { heartbeat_age_ms } | Frame::Pong { heartbeat_age_ms } => {
                        eprintln!("[waiter] keepalive (db heartbeat age {heartbeat_age_ms} ms)");
                        if heartbeat_age_ms > args.stale_heartbeat_ms {
                            eprintln!(
                                "[waiter] HUNG: holder DB heartbeat stale ({heartbeat_age_ms} ms)"
                            );
                            return 4;
                        }
                    }
                    Frame::Timeout => {
                        eprintln!("[waiter] idle-timeout (no message)");
                        return 2;
                    }
                    Frame::Message {
                        id,
                        address,
                        body,
                        attention,
                        sent_at_ms,
                        buffered_at_ms,
                    } => {
                        let waiter_exit_ms = telex_spike::now_ms();
                        println!(
                            "{}",
                            serde_json::json!({
                                "id": id, "address": address,
                                "body": body, "attention": attention,
                                "sent_at_ms": sent_at_ms,
                                "buffered_at_ms": buffered_at_ms,
                                "waiter_exit_ms": waiter_exit_ms,
                                // decomposed legs (ms):
                                "backend_ms": buffered_at_ms - sent_at_ms,
                                "holder_to_exit_ms": waiter_exit_ms - buffered_at_ms,
                                "send_to_exit_ms": waiter_exit_ms - sent_at_ms
                            })
                        );
                        eprintln!(
                            "[waiter] delivered id={id} backend={}ms holder_to_exit={}ms send_to_exit={}ms",
                            buffered_at_ms - sent_at_ms,
                            waiter_exit_ms - buffered_at_ms,
                            waiter_exit_ms - sent_at_ms
                        );
                        return 0;
                    }
                }
            }
        }
        if Instant::now() > overall_deadline {
            eprintln!("[waiter] idle-timeout (overall cap)");
            return 2;
        }
    }
}
