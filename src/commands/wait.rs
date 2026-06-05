//! `telex wait`: the ephemeral delivery client. Connects to a running holder over local
//! IPC, blocks until an actionable message is ready, prints it as JSON, and exits. The
//! exit is what hands the message to the agent.
//!
//! Exit codes (the client contract):
//!   0 = message delivered (printed as JSON on stdout)
//!   2 = idle timeout (no message within --timeout-ms)
//!   3 = holder gone (connect/read failed or EOF)
//!   4 = holder hung (no frame within --hang-ms, or DB heartbeat gone stale)

use anyhow::Result;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::cli::{Ctx, WaitArgs};
use crate::ipc::{self, Frame, Request};
use crate::model::now_ms;

pub async fn run(ctx: &Ctx, args: WaitArgs) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;

    let stream = match ipc::connect(&address).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[wait] holder-gone (connect failed): {e}");
            return Ok(3);
        }
    };
    let (read_half, mut write_half) = tokio::io::split(stream);

    let req = Request::Wait {
        address: address.clone(),
        since: args.since,
        timeout_ms: args.timeout_ms,
    };
    let mut line = serde_json::to_string(&req)?;
    line.push('\n');
    if write_half.write_all(line.as_bytes()).await.is_err() {
        eprintln!("[wait] holder-gone (write failed)");
        return Ok(3);
    }
    let _ = write_half.flush().await;

    let mut reader = BufReader::new(read_half);
    let hang = Duration::from_millis(args.hang_ms);

    loop {
        let mut buf = String::new();
        match tokio::time::timeout(hang, reader.read_line(&mut buf)).await {
            Err(_) => {
                eprintln!("[wait] HUNG: no frame within {} ms", args.hang_ms);
                return Ok(4);
            }
            Ok(Ok(0)) => {
                eprintln!("[wait] holder-gone (EOF)");
                return Ok(3);
            }
            Ok(Err(e)) => {
                eprintln!("[wait] holder-gone (read error): {e}");
                return Ok(3);
            }
            Ok(Ok(_)) => {
                let frame: Frame = match serde_json::from_str(buf.trim()) {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("[wait] bad frame: {e} ({})", buf.trim());
                        continue;
                    }
                };
                match frame {
                    Frame::Keepalive { heartbeat_age_ms } | Frame::Pong { heartbeat_age_ms } => {
                        if heartbeat_age_ms > args.stale_heartbeat_ms {
                            eprintln!(
                                "[wait] HUNG: holder DB heartbeat stale ({heartbeat_age_ms} ms)"
                            );
                            return Ok(4);
                        }
                    }
                    Frame::ShuttingDown => {
                        eprintln!("[wait] holder shutting down");
                        return Ok(3);
                    }
                    Frame::Timeout => {
                        eprintln!("[wait] idle-timeout (no message)");
                        return Ok(2);
                    }
                    Frame::Message {
                        id,
                        thread_id,
                        parent_id,
                        from_addr,
                        to_addr,
                        kind,
                        attention,
                        requires_disposition,
                        subject,
                        body,
                        sent_at_ms,
                        buffered_at_ms,
                    } => {
                        let waiter_exit_ms = now_ms();
                        println!(
                            "{}",
                            serde_json::json!({
                                "id": id,
                                "thread_id": thread_id,
                                "parent_id": parent_id,
                                "from": from_addr,
                                "to": to_addr,
                                "kind": kind,
                                "attention": attention,
                                "requires_disposition": requires_disposition,
                                "subject": subject,
                                "body": body,
                                "sent_at_ms": sent_at_ms,
                                "buffered_at_ms": buffered_at_ms,
                                "waiter_exit_ms": waiter_exit_ms,
                                "backend_ms": buffered_at_ms - sent_at_ms,
                                "send_to_exit_ms": waiter_exit_ms - sent_at_ms,
                            })
                        );
                        return Ok(0);
                    }
                }
            }
        }
    }
}
