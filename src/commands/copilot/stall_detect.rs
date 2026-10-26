//! Read-only group policy. Watcher owns scheduling, delivery and state commits.

use anyhow::{bail, Context, Result};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;

use super::session_state::{classify_local, parse_session_id, Classification, SessionState};
use crate::backend::Backend;
use crate::watcher_protocol::{
    self as protocol, DetectorEvent, DetectorRequest, DetectorResult, Outcome,
};

const MAX_MEMBERS: usize = 32;
const MAX_MEMBER_BYTES: usize = 4096;
const MAX_INBOX_ITEMS: usize = 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Parameters {
    group_directory: PathBuf,
    backend_profile: String,
    watcher_executable: PathBuf,
    watcher_registry: PathBuf,
    grace_seconds: u32,
    #[serde(default = "default_poll_interval")]
    poll_interval_seconds: u32,
}

fn default_poll_interval() -> u32 {
    30
}

impl Parameters {
    fn validate(&self) -> Result<()> {
        if !self.group_directory.is_absolute()
            || !self.watcher_executable.is_absolute()
            || !self.watcher_registry.is_absolute()
        {
            bail!("groupDirectory, watcherExecutable and watcherRegistry must be absolute");
        }
        if self.backend_profile.is_empty()
            || !(1..=86_400).contains(&self.grace_seconds)
            || !(1..=86_400).contains(&self.poll_interval_seconds)
        {
            bail!(
                "backendProfile is required; graceSeconds and pollIntervalSeconds must be 1-86400"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum MemberState {
    Active,
    Parked,
    OperatorHeld,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Member {
    schema_version: u32,
    session_id: String,
    address: String,
    state: MemberState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    member: Member,
    session: Option<Classification>,
    wake_sources: Option<WakeSources>,
    background_work: &'static str,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WakeSources {
    unacknowledged_messages: i64,
    deferred_messages: usize,
    armed_watches: Vec<String>,
}

impl WakeSources {
    fn present(&self) -> bool {
        self.unacknowledged_messages > 0
            || self.deferred_messages > 0
            || !self.armed_watches.is_empty()
    }
}

fn bounded_read(path: &Path, cap: usize) -> Result<Vec<u8>> {
    let file = File::open(path).context("cannot read observation file")?;
    if !file.metadata()?.is_file() {
        bail!("observation path must be a regular file");
    }
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > cap {
        bail!("observation file exceeds byte limit");
    }
    Ok(bytes)
}

fn read_members(directory: &Path) -> Result<Vec<Member>> {
    if !directory.is_absolute() {
        bail!("groupDirectory must be absolute");
    }
    let mut members = Vec::new();
    let mut addresses = BTreeSet::new();
    let mut entries = 0;
    for entry in std::fs::read_dir(directory).context("cannot read group directory")? {
        let entry = entry?;
        entries += 1;
        if entries > MAX_MEMBERS * 2 {
            bail!("group directory entry limit exceeded");
        }
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        if !entry.file_type()?.is_file() {
            bail!("membership entries must be regular files, not directories or symlinks");
        }
        let bytes = bounded_read(&entry.path(), MAX_MEMBER_BYTES)?;
        let member: Member = serde_json::from_value(protocol::parse_json(&bytes)?)
            .context("invalid member record")?;
        parse_session_id(&member.session_id).map_err(anyhow::Error::msg)?;
        if member.schema_version != 1
            || entry.path().file_stem().and_then(|name| name.to_str())
                != Some(member.session_id.as_str())
        {
            bail!("member schemaVersion must be 1 and filename must match sessionId");
        }
        if member.address.is_empty()
            || member.address.len() > 256
            || member
                .address
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
            || member
                .reason
                .as_ref()
                .is_some_and(|reason| reason.len() > 256)
        {
            bail!("member address or reason is invalid or exceeds 256 bytes");
        }
        if !addresses.insert(member.address.clone()) {
            bail!("members must have distinct Telex addresses");
        }
        members.push(member);
        if members.len() > MAX_MEMBERS {
            bail!("group exceeds 32 members");
        }
    }
    members.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    Ok(members)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchEnvelope {
    schema_version: u32,
    command: String,
    ok: bool,
    result: Option<WatchList>,
}

#[derive(Debug, Deserialize)]
struct WatchList {
    r#type: String,
    value: Vec<Watch>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Watch {
    schema_version: u32,
    id: String,
    lifecycle: String,
    registration: WatchRoute,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchRoute {
    schema_version: u32,
    backend_profile: String,
    target: String,
}

async fn read_watches(parameters: &Parameters) -> Result<Vec<Watch>> {
    let mut child = tokio::process::Command::new(&parameters.watcher_executable)
        .args(["--json", "--registry"])
        .arg(&parameters.watcher_registry)
        .args(["list", "--active"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("cannot query Watcher management")?;
    let mut stdout = child
        .stdout
        .take()
        .context("Watcher stdout unavailable")?
        .take(256 * 1024 + 1);
    let mut stderr = child
        .stderr
        .take()
        .context("Watcher stderr unavailable")?
        .take(64 * 1024 + 1);
    let query = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let (out_read, err_read, status) = tokio::join!(
            stdout.read_to_end(&mut out),
            stderr.read_to_end(&mut err),
            child.wait()
        );
        out_read?;
        err_read?;
        if !status?.success() || out.len() > 256 * 1024 || err.len() > 64 * 1024 {
            bail!("Watcher management failed or exceeded output limits");
        }
        parse_watches(&out)
    };
    match tokio::time::timeout(Duration::from_secs(5), query).await {
        Ok(result) => result,
        Err(_) => {
            child
                .kill()
                .await
                .context("cannot stop timed-out Watcher query")?;
            bail!("Watcher management query timed out");
        }
    }
}

fn parse_watches(bytes: &[u8]) -> Result<Vec<Watch>> {
    let envelope: WatchEnvelope = serde_json::from_value(protocol::parse_json(bytes)?)
        .context("invalid Watcher management envelope")?;
    let list = envelope
        .result
        .context("Watcher management result missing")?;
    if envelope.schema_version != 1
        || envelope.command != "list"
        || !envelope.ok
        || list.r#type != "watches"
        || list.value.iter().any(|watch| {
            watch.schema_version != 2
                || watch.registration.schema_version != 2
                || watch.lifecycle != "active"
                || watch.id.is_empty()
        })
    {
        bail!("unsupported or unsuccessful Watcher management observation");
    }
    Ok(list.value)
}

async fn message_wakes(
    backend: &dyn Backend,
    member: &Member,
    pending: &[(String, i64)],
) -> Result<WakeSources> {
    let unacknowledged_messages = pending
        .iter()
        .find(|(address, _)| address == &member.address)
        .map_or(0, |(_, count)| *count);
    if unacknowledged_messages < 0 {
        bail!("invalid unacknowledged message count");
    }
    let inbox = backend
        .inbox(&member.address, true, i64::try_from(MAX_INBOX_ITEMS + 1)?)
        .await?;
    let deferred_messages = inbox
        .iter()
        .filter(|item| {
            item.primary_to == member.address
                && item.latest_disposition.as_deref() == Some("deferred")
        })
        .count();
    // A full suffix cannot prove the absence of an older deferred message.
    if inbox.len() > MAX_INBOX_ITEMS && deferred_messages == 0 && unacknowledged_messages == 0 {
        bail!("inbox history exceeds observation limit; deferred absence is unobserved");
    }
    Ok(WakeSources {
        unacknowledged_messages,
        deferred_messages,
        armed_watches: Vec::new(),
    })
}

async fn observe(parameters: &Parameters, requesting_watch_id: &str) -> Result<Vec<Observation>> {
    let members = read_members(&parameters.group_directory)?;
    let mut observations = Vec::new();
    if members
        .iter()
        .all(|member| member.state != MemberState::Active)
    {
        return Ok(members
            .into_iter()
            .map(|member| Observation {
                member,
                session: None,
                wake_sources: None,
                background_work: "unobserved",
            })
            .collect());
    }
    let (_, profile) = crate::profiles::resolve(Some(&parameters.backend_profile), None)?;
    let store_key = crate::profiles::store_key(&profile, None);
    let backend = crate::profiles::build_readonly(&profile, None).await?;
    let pending = backend.undelivered_counts().await?;
    let watches = read_watches(parameters).await?;
    for member in &members {
        let (session, wake_sources) = if member.state == MemberState::Active {
            let session = classify_local(&member.session_id)?;
            let mut wakes = message_wakes(backend.as_ref(), member, &pending).await?;
            for watch in watches.iter().filter(|watch| {
                watch.id != requesting_watch_id && watch.registration.target == member.address
            }) {
                let (_, watch_profile) =
                    crate::profiles::resolve(Some(&watch.registration.backend_profile), None)?;
                if crate::profiles::store_key(&watch_profile, None) == store_key {
                    wakes.armed_watches.push(watch.id.clone());
                }
            }
            wakes.armed_watches.sort();
            (Some(session), Some(wakes))
        } else {
            (None, None)
        };
        observations.push(Observation {
            member: member.clone(),
            session,
            wake_sources,
            background_work: "unobserved",
        });
    }
    if read_members(&parameters.group_directory)? != members {
        bail!("membership changed during observation");
    }
    Ok(observations)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Episode {
    schema_version: u32,
    fingerprint: String,
    first_seen_at: i64,
    last_observed_at: i64,
    gap_restarted: bool,
    emitted: bool,
}

fn timestamp_ms(timestamp: &str) -> Result<i64> {
    protocol::validate_timestamp(timestamp)?;
    Ok(DateTime::parse_from_rfc3339(timestamp)?.timestamp_millis())
}

fn degraded() -> DetectorResult {
    DetectorResult {
        schema_version: 2,
        outcome: Outcome::Degraded,
        next_state: None,
        event: None,
    }
}

fn idle(state: Value) -> DetectorResult {
    DetectorResult {
        schema_version: 2,
        outcome: Outcome::Idle,
        next_state: Some(state),
        event: None,
    }
}

fn evaluate(
    request: &DetectorRequest,
    parameters: &Parameters,
    observations: &[Observation],
) -> Result<DetectorResult> {
    let now = timestamp_ms(&request.attempt.now)?;
    let grace = i64::from(parameters.grace_seconds) * 1000;
    let active: Vec<_> = observations
        .iter()
        .filter(|observation| observation.member.state == MemberState::Active)
        .collect();
    if active
        .iter()
        .any(|observation| observation.session.is_none() || observation.wake_sources.is_none())
    {
        eprintln!("telex copilot stall-detect: incomplete active-member observation");
        return Ok(degraded());
    }
    if active.iter().any(|observation| {
        observation
            .session
            .as_ref()
            .and_then(|session| session.activity.as_ref())
            .is_some_and(|activity| !timestamp_ms(&activity.timestamp).is_ok_and(|at| at <= now))
    }) {
        eprintln!("telex copilot stall-detect: invalid or future session evidence timestamp");
        return Ok(degraded());
    }
    let unknown = active.iter().any(|observation| {
        observation
            .session
            .as_ref()
            .is_none_or(|session| session.state == SessionState::Unknown)
    });
    let quiet = !active.is_empty()
        && active.iter().all(|observation| {
            observation.session.as_ref().is_some_and(|session| {
                matches!(
                    session.state,
                    SessionState::Idle | SessionState::NotLoaded | SessionState::Unknown
                )
            }) && observation
                .wake_sources
                .as_ref()
                .is_some_and(|wakes| !wakes.present())
        });
    if !quiet {
        return Ok(if unknown {
            degraded()
        } else {
            idle(Value::Null)
        });
    }
    if unknown
        && !active.iter().all(|observation| {
            let session = observation
                .session
                .as_ref()
                .expect("quiet requires session");
            match &session.activity {
                Some(activity) => {
                    timestamp_ms(&activity.timestamp).is_ok_and(|at| now.saturating_sub(at) > grace)
                }
                None => session.state != SessionState::Unknown,
            }
        })
    {
        return Ok(degraded());
    }
    let fingerprint = protocol::hash_value(&json!({
        "parameters":request.watch.parameters,
        "members":observations,
        "activatedAt":request.watch.activated_at,
    }))?;
    let previous: Option<Episode> = if request.state.is_null() {
        None
    } else {
        Some(
            serde_json::from_value(request.state.clone())
                .context("invalid detector episode state")?,
        )
    };
    let mut episode = match previous {
        Some(episode) if episode.schema_version == 1 && episode.fingerprint == fingerprint => {
            episode
        }
        Some(episode) if episode.schema_version != 1 => bail!("unsupported episode schemaVersion"),
        _ => Episode {
            schema_version: 1,
            fingerprint,
            first_seen_at: now,
            last_observed_at: now,
            gap_restarted: false,
            emitted: false,
        },
    };
    if now < episode.last_observed_at || episode.first_seen_at > episode.last_observed_at {
        eprintln!("telex copilot stall-detect: observation clock moved backward; grace restarted");
        episode.first_seen_at = now;
        episode.last_observed_at = now;
    }
    let freshness = i64::from(parameters.poll_interval_seconds) * 2000;
    if !episode.emitted
        && !episode.gap_restarted
        && now.saturating_sub(episode.last_observed_at) > freshness
    {
        episode.first_seen_at = now;
        episode.gap_restarted = true;
    }
    episode.last_observed_at = now;
    if episode.emitted {
        return Ok(if unknown {
            degraded()
        } else {
            idle(serde_json::to_value(episode)?)
        });
    }
    if !unknown && now.saturating_sub(episode.first_seen_at) <= grace {
        return Ok(idle(serde_json::to_value(episode)?));
    }
    episode.emitted = true;
    let kind = if unknown {
        "copilot.group.attention-needed"
    } else {
        "copilot.group.stalled"
    };
    let snapshot = json!({
        "schemaVersion":1,
        "groupDirectory":parameters.group_directory,
        "degraded":unknown,
        "members":observations,
        "firstSeenAt":episode.first_seen_at,
        "observedAt":request.attempt.now,
        "fingerprint":episode.fingerprint,
    });
    Ok(DetectorResult {
        schema_version: 2,
        outcome: Outcome::Event,
        next_state: Some(serde_json::to_value(&episode)?),
        event: Some(DetectorEvent {
            kind: kind.into(),
            subject: if unknown {
                "Copilot group needs attention (degraded observation)"
            } else {
                "Copilot group stalled"
            }
            .into(),
            body: serde_json::to_string(&snapshot)?,
            metadata: json!({"degraded":unknown, "fingerprint":episode.fingerprint}),
        }),
    })
}

pub async fn run() -> Result<i32> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut input)?;
    if input.len() > 1024 * 1024 {
        bail!("detector request exceeds byte limit");
    }
    let request = protocol::parse_request(&input)?;
    let parameters: Parameters = serde_json::from_value(request.watch.parameters.clone())
        .context("invalid stall detector parameters")?;
    parameters.validate()?;
    let result = match tokio::time::timeout(
        Duration::from_secs(15),
        observe(&parameters, &request.watch.id),
    )
    .await
    {
        Ok(Ok(observations)) => evaluate(&request, &parameters, &observations)?,
        Ok(Err(error)) => {
            eprintln!("telex copilot stall-detect: {error:#}");
            degraded()
        }
        Err(_) => {
            eprintln!("telex copilot stall-detect: observation timed out");
            degraded()
        }
    };
    let result = match protocol::validate_result(&result) {
        Ok(()) => result,
        Err(error) => {
            eprintln!("telex copilot stall-detect: result rejected: {error}");
            degraded()
        }
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(0)
}

#[cfg(test)]
#[path = "stall_detect_tests.rs"]
mod tests;
