//! Desktop-specific glue. The app schedules turns; this command only talks to Telex.
use crate::{
    cli::Ctx,
    daemon,
    daemon_ipc::{Request, Response},
    scheduled::{self, Action},
};
use anyhow::{bail, ensure, Context, Result};
use clap::Subcommand;
use serde_json::json;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the installable Codex station skill.
    Skill,
    /// Prepare a same-chat scheduled station.
    Bind {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_parser=interval)]
        every: u32,
    },
    /// Activate the schedule after verifying it in the desktop app.
    Confirm {
        #[arg(long)]
        binding: String,
        #[arg(long)]
        automation: String,
    },
    /// Scan once for the next unconsumed delivery.
    Check {
        #[arg(long)]
        binding: String,
    },
    /// Acknowledge the exact delivery returned by check.
    Ack {
        #[arg(long)]
        binding: String,
        #[arg(long)]
        receipt: String,
    },
    /// Send or recover the single threaded reply for this delivery.
    Reply {
        #[arg(long)]
        binding: String,
        #[arg(long)]
        receipt: String,
        #[arg(long)]
        body_file: String,
    },
    /// Show the schedule and last observed check.
    Status {
        #[arg(long)]
        binding: String,
    },
    /// Disable a binding before removing its app heartbeat.
    Unbind {
        #[arg(long)]
        binding: String,
    },
}

fn interval(s: &str) -> std::result::Result<u32, String> {
    let (digits, scale) = if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else {
        (s.strip_suffix('s').unwrap_or(s), 1)
    };
    digits
        .parse::<u32>()
        .ok()
        .and_then(|n| n.checked_mul(scale))
        .filter(|n| (60..=86400).contains(n))
        .ok_or_else(|| "expected an interval between 1m and 24h".to_owned())
}

fn session_id() -> Result<String> {
    let id = std::env::var("CODEX_THREAD_ID").context(
        "run this command inside the destination local Codex thread (CODEX_THREAD_ID is required)",
    )?;
    ensure!(
        id.len() == 36
            && id
                .bytes()
                .enumerate()
                .all(|(i, b)| if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }),
        "invalid CODEX_THREAD_ID"
    );
    Ok(id.to_ascii_lowercase())
}

pub async fn run(ctx: &Ctx, cmd: Command) -> Result<i32> {
    match execute(ctx, cmd).await {
        Ok(code) => Ok(code),
        Err(error) => {
            println!(
                "{}",
                json!({"result":"error","code":"ClientError","message":format!("{error:#}")})
            );
            Ok(1)
        }
    }
}

async fn execute(ctx: &Ctx, cmd: Command) -> Result<i32> {
    if matches!(cmd, Command::Skill) {
        print!("{}", include_str!("../../codex/skills/telex/SKILL.md"));
        return Ok(0);
    }
    let session = session_id()?;
    let receipt = scheduled::nonce()?;
    let (store_key, address, binding, action) = if let Command::Bind {
        session: expected,
        every,
    } = cmd
    {
        ensure!(
            expected.is_none_or(|id| id.eq_ignore_ascii_case(&session)),
            "requested session differs from current thread"
        );
        let (name, profile) = ctx.resolved()?;
        let db = ctx.cfg.db_override.as_deref();
        // An implicit SQLite store has no configured profile to resolve in a later turn.
        let selector = if db.is_none() && crate::profiles::load()?.backends.contains_key(&name) {
            Some(name)
        } else {
            None
        };
        (
            crate::profiles::store_key(&profile, db),
            ctx.cfg.require_address(&ctx.address)?,
            None,
            Action::Prepare {
                interval_seconds: every,
                selector,
            },
        )
    } else {
        let (id, action) = match cmd {
            Command::Confirm {
                binding,
                automation,
            } => (binding, Action::Confirm { automation }),
            Command::Check { binding } => (
                binding,
                Action::Check {
                    receipt: receipt.clone(),
                },
            ),
            Command::Ack { binding, receipt } => (binding, Action::Ack { receipt }),
            Command::Reply {
                binding,
                receipt,
                body_file,
            } => {
                let body = super::resolve_body(None, Some(body_file), false)?;
                (binding, Action::Reply { receipt, body })
            }
            Command::Status { binding } => (binding, Action::Status),
            Command::Unbind { binding } => (binding, Action::Disable),
            _ => unreachable!(),
        };
        let paths = daemon::DaemonPaths::current()?;
        let Some(b) = scheduled::load(&paths, &session)?.filter(|b| b.generation == id) else {
            println!("{}", json!({"result":"inactive"}));
            return Ok(5);
        };
        if !matches!(action, Action::Disable | Action::Status) {
            if let Some(name) = b.selector.as_deref() {
                let (_, profile) = crate::profiles::resolve(Some(name), None)?;
                ensure!(
                    crate::profiles::store_key(&profile, None) == b.store_key,
                    "backend profile changed; explicitly rebind the station"
                )
            }
        }
        (b.store_key, b.address, Some(id), action)
    };
    let mut client = daemon::connect_or_spawn(&store_key).await?;
    ensure!(
        client
            .ack
            .capabilities
            .iter()
            .any(|s| s == scheduled::CAPABILITY),
        "upgrade the Telex daemon for scheduled checks"
    );
    let preparing = matches!(action, Action::Prepare { .. });
    let response = client
        .request(&Request::Scheduled {
            store_key,
            session_id: session,
            address,
            binding,
            action,
        })
        .await?;
    let (mut value, code) = match response {
        Response::Scheduled { result } => {
            let code = match result["result"].as_str() {
                Some("busy") => 2,
                Some("inactive") => 5,
                _ => 0,
            };
            (result, code)
        }
        msg @ Response::Message { .. } => (
            json!({"result":"delivery","receipt":receipt,"message":msg}),
            0,
        ),
        Response::Sent { receipt } => (json!({"result":"replied","reply":receipt}), 0),
        v @ Response::DeliveryQuarantined { .. } => (json!({"result":"quarantined","detail":v}), 6),
        Response::Error { code, message, .. } => {
            (json!({"result":"error","code":code,"message":message}), 1)
        }
        other => bail!("unexpected scheduled response: {other:?}"),
    };
    if preparing && value["result"] == "binding" {
        value["check_argv"] = json!([
            std::env::current_exe()?,
            "codex",
            "check",
            "--binding",
            value["binding"]
        ]);
    }
    println!("{}", serde_json::to_string(&value)?);
    Ok(code)
}
