#![cfg(all(feature = "postgres", feature = "sqlite"))]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use telex::application_client::{
    ApplicationClient, ApplicationClientConfig, ApplicationClientError, ApplicationResponsibility,
    LogicalStoreId, OperationId, PayloadIdentity, RecordedOperationOutcome, RecoveryHandle,
    SendRequest,
};
use telex::backend::postgres::{make_tls, sanitize_ident, PgBackend};
use telex::backend::Backend;
use telex::daemon::test_support::{registered_epoch, send_request, TestDaemon};
use telex::daemon_ipc::{self as proto, Request, Response, WatchPidSpec};
use telex::model::{
    now_ms, ApplicationMessageOperation, ApplicationOperationBegin, ApplicationRecordScope,
    Attention, DeliveryOutcome, Disposition, NewApplicationOperation, NewMessage, RetentionPolicy,
};
use telex::profiles::{self, BackendProfile, ConfigFile};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn application_client_schema_v3_operation_smoke() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let Some(url) = pg_url_or_skip("application_client_schema_v3_operation_smoke") else {
        return;
    };
    let cfg = pg_config(&url);
    let schema = sanitize_ident(&format!(
        "telex_app_client_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .unwrap();
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .unwrap();
    let backend = PgBackend::connect_with(cfg.clone(), Some(&schema))
        .await
        .unwrap();
    backend.init_schema().await.unwrap();
    let store_id = backend.logical_store_id().await.unwrap();
    let operation = NewApplicationOperation {
        logical_store_id: store_id,
        application_responsibility: "postgres-smoke".into(),
        operation_id: "operation-1".into(),
        operation_kind: "send".into(),
        sender: "postgres:sender".into(),
        recipients_json: r#"["postgres:target"]"#.into(),
        payload_fingerprint: "fingerprint".into(),
        retry_budget: 1,
        created_at_ms: now_ms(),
    };
    assert!(matches!(
        backend
            .begin_application_operation(&operation)
            .await
            .unwrap(),
        ApplicationOperationBegin::Started(_)
    ));
    assert!(matches!(
        backend
            .begin_application_operation(&operation)
            .await
            .unwrap(),
        ApplicationOperationBegin::Replay(existing)
            if existing.payload_fingerprint == operation.payload_fingerprint
    ));
    let mut mismatched_operation = operation.clone();
    mismatched_operation.payload_fingerprint = "different-fingerprint".into();
    assert!(matches!(
        backend
            .begin_application_operation(&mismatched_operation)
            .await
            .unwrap(),
        ApplicationOperationBegin::FingerprintMismatch(existing)
            if existing.payload_fingerprint == operation.payload_fingerprint
    ));
    assert_eq!(
        backend
            .complete_application_operation(
                &operation.logical_store_id,
                &operation.application_responsibility,
                &operation.operation_id,
                "accepted",
                Some("{}"),
                None,
            )
            .await
            .unwrap()
            .state,
        "accepted"
    );
    let parent = backend
        .insert_message(&NewMessage {
            from_addr: Some("postgres:target".into()),
            to_addr: "postgres:sender".into(),
            kind: "request".into(),
            attention: Attention::Background,
            body: "parent".into(),
            sent_at_ms: now_ms(),
            ..Default::default()
        })
        .await
        .unwrap();
    let reply_operation = NewApplicationOperation {
        logical_store_id: operation.logical_store_id.clone(),
        application_responsibility: "postgres-smoke".into(),
        operation_id: "reply-operation".into(),
        operation_kind: "reply".into(),
        sender: "postgres:sender".into(),
        recipients_json: format!("[{},[]]", parent.id),
        payload_fingerprint: "reply-fingerprint".into(),
        retry_budget: 1,
        created_at_ms: now_ms(),
    };
    assert!(matches!(
        backend
            .begin_application_operation(&reply_operation)
            .await
            .unwrap(),
        ApplicationOperationBegin::Started(_)
    ));
    let metadata = r#"{"urn:test:opaque":{"value":"postgres"}}"#;
    let reply = backend
        .insert_application_message(
            &NewMessage {
                parent_id: Some(parent.id),
                from_addr: Some("postgres:sender".into()),
                to_addr: "postgres:target".into(),
                kind: "reply".into(),
                attention: Attention::Background,
                body: "reply".into(),
                metadata: Some(metadata.into()),
                sent_at_ms: now_ms(),
                ..Default::default()
            },
            &ApplicationMessageOperation {
                logical_store_id: reply_operation.logical_store_id.clone(),
                application_responsibility: reply_operation.application_responsibility.clone(),
                operation_id: reply_operation.operation_id.clone(),
                payload_fingerprint: reply_operation.payload_fingerprint.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(reply.metadata.as_deref(), Some(metadata));
    assert!(backend
        .thread_messages(parent.thread_id)
        .await
        .unwrap()
        .iter()
        .any(|message| message.id == reply.id && message.metadata.as_deref() == Some(metadata)));

    let comparable_digest = "c".repeat(64);
    let pending_operation = NewApplicationOperation {
        logical_store_id: operation.logical_store_id.clone(),
        application_responsibility: "postgres-public-client".into(),
        operation_id: "public-reconcile".into(),
        operation_kind: "send".into(),
        sender: "postgres:sender".into(),
        recipients_json: r#"["postgres:target"]"#.into(),
        payload_fingerprint: comparable_digest.clone(),
        retry_budget: 1,
        created_at_ms: now_ms(),
    };
    assert!(matches!(
        backend
            .begin_application_operation(&pending_operation)
            .await
            .unwrap(),
        ApplicationOperationBegin::Started(_)
    ));
    let other_responsibility_operation = NewApplicationOperation {
        application_responsibility: "postgres-other-client".into(),
        ..pending_operation.clone()
    };
    assert!(matches!(
        backend
            .begin_application_operation(&other_responsibility_operation)
            .await
            .unwrap(),
        ApplicationOperationBegin::Started(_)
    ));
    let shared_operation_deltas: Vec<_> = backend
        .state_deltas(0, 10_000)
        .await
        .unwrap()
        .into_iter()
        .filter(|delta| {
            delta.axis == "operation"
                && delta
                    .payload_json
                    .contains("\"operation_id\":\"public-reconcile\"")
        })
        .collect();
    assert_eq!(shared_operation_deltas.len(), 2);
    assert_ne!(
        shared_operation_deltas[0].entity_id,
        shared_operation_deltas[1].entity_id
    );
    assert!(shared_operation_deltas.iter().any(|delta| {
        delta
            .payload_json
            .contains("\"application_responsibility\":\"postgres-public-client\"")
    }));
    assert!(shared_operation_deltas.iter().any(|delta| {
        delta
            .payload_json
            .contains("\"application_responsibility\":\"postgres-other-client\"")
    }));
    backend
        .insert_application_message(
            &NewMessage {
                from_addr: Some("postgres:sender".into()),
                to_addr: "postgres:target".into(),
                kind: "note".into(),
                attention: Attention::Background,
                body: "public reconcile".into(),
                sent_at_ms: now_ms(),
                ..Default::default()
            },
            &ApplicationMessageOperation {
                logical_store_id: pending_operation.logical_store_id.clone(),
                application_responsibility: pending_operation.application_responsibility.clone(),
                operation_id: pending_operation.operation_id.clone(),
                payload_fingerprint: comparable_digest.clone(),
            },
        )
        .await
        .unwrap();
    let config = ConfigFile {
        default: Some("pg-public-client".into()),
        backends: BTreeMap::from([(
            "pg-public-client".into(),
            BackendProfile {
                kind: "postgres".into(),
                path: None,
                url: Some(url.clone()),
                auth: Some("password".into()),
                password_env: None,
                password_command: None,
                schema: Some(schema.clone()),
                entra_cred: None,
                entra_scope: None,
            },
        )]),
    };
    let config_path = write_temp_config("application-client-public", &config);
    let previous_config = std::env::var_os("TELEX_CONFIG");
    std::env::set_var("TELEX_CONFIG", &config_path);
    let client = ApplicationClient::connect(ApplicationClientConfig {
        responsibility: ApplicationResponsibility("postgres-public-client".into()),
        backend: Some("pg-public-client".into()),
        db_override: None,
    })
    .await
    .unwrap();
    let missing_reference = client
        .prepare_send(&SendRequest {
            operation_id: OperationId("postgres-not-recorded".into()),
            sender: "postgres:sender".into(),
            to: "postgres:target".into(),
            cc: Vec::new(),
            kind: "note".into(),
            attention: "background".into(),
            requires_disposition: false,
            subject: None,
            body: "not submitted".into(),
            metadata: None,
            retry_budget: 0,
        })
        .await
        .unwrap();
    assert!(matches!(
        client
            .reconcile_operation(&missing_reference)
            .await
            .unwrap(),
        telex::application_client::OperationReconciliation::NotRecorded(evidence)
            if evidence.operation_id.0 == "postgres-not-recorded"
                && evidence.logical_store_id == *client.logical_store_id()
    ));
    let other_cleanup_operation = NewApplicationOperation {
        logical_store_id: pending_operation.logical_store_id.clone(),
        application_responsibility: "postgres-other-client".into(),
        operation_id: "postgres-other-cleanup-boundary".into(),
        operation_kind: "send".into(),
        sender: "postgres:other".into(),
        recipients_json: r#"["postgres:target"]"#.into(),
        payload_fingerprint: "e".repeat(64),
        retry_budget: 0,
        created_at_ms: 1,
    };
    backend
        .begin_application_operation(&other_cleanup_operation)
        .await
        .unwrap();
    backend
        .complete_application_operation(
            &other_cleanup_operation.logical_store_id,
            &other_cleanup_operation.application_responsibility,
            &other_cleanup_operation.operation_id,
            "rejected",
            Some("{}"),
            None,
        )
        .await
        .unwrap();
    backend
        .cleanup_application_records(
            &ApplicationRecordScope {
                logical_store_id: other_cleanup_operation.logical_store_id.clone(),
                application_responsibility: other_cleanup_operation
                    .application_responsibility
                    .clone(),
            },
            RetentionPolicy {
                completed_before_ms: i64::MAX,
                max_delete: 1,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        client
            .reconcile_operation(&missing_reference)
            .await
            .unwrap(),
        telex::application_client::OperationReconciliation::NotRecorded(_)
    ));
    let cleanup_operation = NewApplicationOperation {
        logical_store_id: pending_operation.logical_store_id.clone(),
        application_responsibility: pending_operation.application_responsibility.clone(),
        operation_id: "postgres-cleanup-boundary".into(),
        operation_kind: "send".into(),
        sender: "postgres:sender".into(),
        recipients_json: r#"["postgres:target"]"#.into(),
        payload_fingerprint: "d".repeat(64),
        retry_budget: 0,
        created_at_ms: 1,
    };
    backend
        .begin_application_operation(&cleanup_operation)
        .await
        .unwrap();
    backend
        .complete_application_operation(
            &cleanup_operation.logical_store_id,
            &cleanup_operation.application_responsibility,
            &cleanup_operation.operation_id,
            "rejected",
            Some("{}"),
            None,
        )
        .await
        .unwrap();
    backend
        .cleanup_application_records(
            &ApplicationRecordScope {
                logical_store_id: cleanup_operation.logical_store_id.clone(),
                application_responsibility: cleanup_operation.application_responsibility.clone(),
            },
            RetentionPolicy {
                completed_before_ms: i64::MAX,
                max_delete: 1,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        client
            .reconcile_operation(&missing_reference)
            .await
            .unwrap(),
        telex::application_client::OperationReconciliation::RetentionBoundaryCrossed {
            staged_generation: Some(0),
            current_generation: 1,
        }
    ));
    let noncomparable = RecoveryHandle {
        logical_store_id: LogicalStoreId(pending_operation.logical_store_id.clone()),
        responsibility: ApplicationResponsibility(
            pending_operation.application_responsibility.clone(),
        ),
        operation_id: OperationId(pending_operation.operation_id.clone()),
        payload_identity: PayloadIdentity {
            algorithm: "sha256".into(),
            digest: comparable_digest.clone(),
            comparable: false,
        },
        retention_generation: None,
    };
    assert!(matches!(
        client.reconcile_operation(&noncomparable).await,
        Err(ApplicationClientError::OperationMismatch { .. })
    ));
    assert_eq!(
        backend
            .application_operation(
                &pending_operation.logical_store_id,
                &pending_operation.application_responsibility,
                &pending_operation.operation_id,
            )
            .await
            .unwrap()
            .unwrap()
            .state,
        "pending"
    );
    let comparable = RecoveryHandle {
        payload_identity: PayloadIdentity {
            algorithm: "sha256".into(),
            digest: comparable_digest,
            comparable: true,
        },
        ..noncomparable
    };
    let telex::application_client::OperationReconciliation::Recorded(reconciled) =
        client.reconcile_operation(&comparable).await.unwrap()
    else {
        panic!("expected recorded operation");
    };
    assert!(matches!(
        reconciled.outcome,
        RecordedOperationOutcome::Accepted(_)
    ));

    let daemon = TestDaemon::new("pg-oversized-delivery-progress");
    let store_key = profiles::store_key(config.backends.get("pg-public-client").unwrap(), None);
    assert!(matches!(
        daemon
            .register(&store_key, "pg-receiver-session", "pg-frame-recipient")
            .await,
        Response::Registered { .. }
    ));
    let oversized_id = backend
        .insert_message(&NewMessage {
            from_addr: Some("pg-frame-sender".into()),
            to_addr: "pg-frame-recipient".into(),
            kind: "note".into(),
            attention: Attention::Background,
            body: "x".repeat(proto::MAX_JSONL_FRAME_BYTES + 1),
            sent_at_ms: now_ms(),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    let following_id = backend
        .insert_message(&NewMessage {
            from_addr: Some("pg-frame-sender".into()),
            to_addr: "pg-frame-recipient".into(),
            kind: "note".into(),
            attention: Attention::Background,
            body: "following".into(),
            sent_at_ms: now_ms(),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    assert!(matches!(
        daemon
            .wait(
                &store_key,
                "pg-receiver-session",
                "pg-frame-recipient",
                1_000,
            )
            .await,
        Response::DeliveryQuarantined {
            message_id,
            ref recipient,
            serialized_bytes,
            max_bytes,
            may_continue: true,
        } if message_id == oversized_id
            && recipient == "pg-frame-recipient"
            && serialized_bytes > max_bytes
    ));
    assert!(backend
        .dispositions_for(oversized_id)
        .await
        .unwrap()
        .iter()
        .any(|disposition| {
            disposition.recipient == "pg-frame-recipient"
                && disposition.state == Disposition::Rejected.as_str()
                && disposition.by_principal.as_deref() == Some("daemon")
                && disposition.origin.as_deref() == Some("daemon-quarantine")
                && disposition.note.as_deref().is_some_and(|note| {
                    note.contains("serialized_bytes=") && note.contains("max_bytes=")
                })
        }));
    let quarantine_deltas = backend.state_delta_page(0, 1_000).await.unwrap();
    for kind in ["acknowledgment", "disposition"] {
        assert!(quarantine_deltas.deltas.iter().any(|delta| {
            delta.axis == kind
                && delta
                    .payload_json
                    .contains("\"evidence\":\"daemon-quarantine\"")
                && delta.payload_json.contains("\"by_principal\":\"daemon\"")
        }));
    }
    assert!(matches!(
        daemon
            .request(Request::Detach {
                store_key: store_key.clone(),
                session_id: "pg-receiver-session".into(),
                address: "pg-frame-recipient".into(),
            })
            .await,
        Response::Ack { .. }
    ));
    let original_instance_id = daemon.instance_id().to_string();
    drop(daemon);
    let restarted = TestDaemon::new("pg-oversized-delivery-progress-restart");
    assert_ne!(restarted.instance_id(), original_instance_id);
    assert!(matches!(
        restarted
            .register(&store_key, "pg-receiver-session", "pg-frame-recipient")
            .await,
        Response::Registered { .. }
    ));
    let restarted_backend = restarted.backend(&store_key).await.unwrap();
    assert!(restarted_backend
        .dispositions_for(oversized_id)
        .await
        .unwrap()
        .iter()
        .any(|disposition| {
            disposition.recipient == "pg-frame-recipient"
                && disposition.state == Disposition::Rejected.as_str()
                && disposition.by_principal.as_deref() == Some("daemon")
                && disposition.origin.as_deref() == Some("daemon-quarantine")
        }));
    assert!(matches!(
        restarted
            .wait(
                &store_key,
                "pg-receiver-session",
                "pg-frame-recipient",
                1_000,
            )
            .await,
        Response::Message { id, .. } if id == following_id
    ));

    for (suffix, version) in [("v3_no_origin", 3), ("v2_upgrade", 2)] {
        let repair_schema = sanitize_ident(&format!(
            "tx_or_{}_{}_{}",
            suffix,
            std::process::id(),
            now_ms()
        ))
        .unwrap();
        admin_exec(
            &cfg,
            &format!(
                "CREATE SCHEMA {repair_schema};
                 CREATE TABLE {repair_schema}.dispositions (
                    id bigserial PRIMARY KEY,
                    message_id bigint NOT NULL,
                    recipient text NOT NULL,
                    state text NOT NULL,
                    note text,
                    by_principal text,
                    at_ms bigint NOT NULL
                 );
                 CREATE TABLE {repair_schema}.telex_schema_version (
                    singleton integer NOT NULL DEFAULT 1 UNIQUE,
                    version bigint NOT NULL
                 );
                 INSERT INTO {repair_schema}.telex_schema_version(singleton, version)
                 VALUES (1, {version});"
            ),
        )
        .await
        .unwrap();
        let repaired = PgBackend::connect_with(cfg.clone(), Some(&repair_schema))
            .await
            .unwrap();
        repaired.init_schema().await.unwrap();
        repaired
            .insert_disposition(99, "recipient", "handled", None, Some("application"))
            .await
            .unwrap();
        assert_eq!(repaired.dispositions_for(99).await.unwrap()[0].origin, None);
        drop(repaired);
        admin_exec(
            &cfg,
            &format!("DROP SCHEMA IF EXISTS {repair_schema} CASCADE"),
        )
        .await
        .unwrap();
    }
    restore_env("TELEX_CONFIG", previous_config);
    drop(client);
    drop(backend);
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .unwrap();
}

fn pg_url_or_skip(test_name: &str) -> Option<String> {
    let require = std::env::var("TELEX_PG_REQUIRE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    match std::env::var("TELEX_PG_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            assert!(
                !require,
                "TELEX_PG_REQUIRE is set but TELEX_PG_URL is unset/empty; refusing to skip {test_name}"
            );
            eprintln!("[daemon-postgres] TELEX_PG_URL not set; skipping {test_name}");
            None
        }
    }
}

fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

async fn admin_exec(cfg: &tokio_postgres::Config, sql: &str) -> anyhow::Result<()> {
    let (client, connection) = cfg.connect(make_tls()?).await?;
    let handle = tokio::spawn(async move {
        let _ = connection.await;
    });
    let res = client.batch_execute(sql).await;
    drop(client);
    let _ = handle.await;
    res?;
    Ok(())
}

fn pg_config(url: &str) -> tokio_postgres::Config {
    let mut cfg: tokio_postgres::Config = url
        .parse()
        .expect("TELEX_PG_URL must be a libpq URI or key=value DSN");
    if let Ok(pw) = std::env::var("TELEX_PG_PASSWORD") {
        if !pw.is_empty() {
            cfg.password(pw);
        }
    }
    cfg
}

fn pg_url_with_application_name(url: &str, application_name: &str) -> String {
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        let separator = if url.contains('?') { '&' } else { '?' };
        format!("{url}{separator}application_name={application_name}")
    } else {
        format!("{url} application_name={application_name}")
    }
}

fn write_temp_config(name: &str, config: &ConfigFile) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "telex-daemon-pg-{name}-config-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&root).expect("create temp config dir");
    let config_path = root.join("config.toml");
    std::fs::write(
        &config_path,
        toml::to_string_pretty(config).expect("serialize config"),
    )
    .expect("write temp config");
    config_path
}

async fn insert_message(backend: &Arc<dyn Backend>, to: &str) -> i64 {
    backend
        .insert_message(&NewMessage {
            parent_id: None,
            from_addr: Some("sender".to_string()),
            to_addr: to.to_string(),
            cc: None,
            kind: "note".to_string(),
            attention: Attention::Background,
            requires_disposition: false,
            subject: None,
            body: "hello from postgres daemon test".to_string(),
            metadata: None,
            sent_at_ms: now_ms(),
        })
        .await
        .expect("insert message")
        .id
}

async fn insert_cc_message(backend: &Arc<dyn Backend>, to: &str, cc: &str) -> i64 {
    backend
        .insert_message(&NewMessage {
            parent_id: None,
            from_addr: Some("sender".to_string()),
            to_addr: to.to_string(),
            cc: Some(cc.to_string()),
            kind: "note".to_string(),
            attention: Attention::Background,
            requires_disposition: false,
            subject: None,
            body: "hello cc from postgres daemon test".to_string(),
            metadata: None,
            sent_at_ms: now_ms(),
        })
        .await
        .expect("insert cc message")
        .id
}

fn record_stdin_argv(path: &std::path::Path) -> Vec<String> {
    let path = path.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        let escaped = path.replace('\'', "''");
        vec![
            "powershell".into(),
            "-NoProfile".into(),
            "-Command".into(),
            format!(
                "[IO.File]::WriteAllText('{escaped}', [Console]::In.ReadToEnd(), [Text.UTF8Encoding]::new($false))"
            ),
        ]
    }
    #[cfg(unix)]
    {
        vec!["tee".into(), path]
    }
}

fn fail_first_then_record_argv(root: &std::path::Path) -> Vec<String> {
    std::fs::create_dir_all(root).expect("create handler root");
    #[cfg(windows)]
    {
        let script = root.join("handler.ps1");
        std::fs::write(
            &script,
            r#"
param([string]$Root)
New-Item -ItemType Directory -Force -Path $Root | Out-Null
$inputText = [Console]::In.ReadToEnd()
if ($inputText -match '"body":"first cc"') {
  $countPath = Join-Path $Root 'first.count'
  $count = 0
  if (Test-Path -LiteralPath $countPath) {
    $count = [int]((Get-Content -LiteralPath $countPath -Raw).Trim())
  }
  $count += 1
  Set-Content -LiteralPath $countPath -Value $count -Encoding utf8
  $attemptPath = Join-Path $Root "first-$count.json"
  [IO.File]::WriteAllText($attemptPath, $inputText, [Text.UTF8Encoding]::new($false))
  if ($count -eq 1) { exit 1 }
  Copy-Item -LiteralPath $attemptPath -Destination (Join-Path $Root 'first-retry.json') -Force
  exit 0
}
if ($inputText -match '"body":"second cc"') {
  [IO.File]::WriteAllText((Join-Path $Root 'second.json'), $inputText, [Text.UTF8Encoding]::new($false))
  exit 0
}
[IO.File]::WriteAllText((Join-Path $Root 'unexpected.json'), $inputText, [Text.UTF8Encoding]::new($false))
exit 0
"#,
        )
        .expect("write handler script");
        vec![
            "powershell".into(),
            "-NoProfile".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-File".into(),
            script.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        ]
    }
    #[cfg(unix)]
    {
        let script = root.join("handler.sh");
        std::fs::write(
            &script,
            r#"
root="$1"
mkdir -p "$root"
input="$(cat)"
if printf '%s' "$input" | grep -q '"body":"first cc"'; then
  count_file="$root/first.count"
  count=0
  if [ -f "$count_file" ]; then
    count="$(cat "$count_file")"
  fi
  count=$((count + 1))
  printf '%s\n' "$count" > "$count_file"
  attempt_path="$root/first-$count.json"
  printf '%s\n' "$input" > "$attempt_path"
  if [ "$count" -eq 1 ]; then
    exit 1
  fi
  cp "$attempt_path" "$root/first-retry.json"
  exit 0
fi
if printf '%s' "$input" | grep -q '"body":"second cc"'; then
  printf '%s\n' "$input" > "$root/second.json"
  exit 0
fi
printf '%s\n' "$input" > "$root/unexpected.json"
exit 0
"#,
        )
        .expect("write handler script");
        vec![
            "sh".into(),
            script.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        ]
    }
}

#[tokio::test]
async fn postgres_application_detach_intent_round_trip() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let Some(url) = pg_url_or_skip("postgres_application_detach_intent_round_trip") else {
        return;
    };
    let cfg = pg_config(&url);
    let schema = sanitize_ident(&format!(
        "telex_app_detach_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .unwrap();
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .unwrap();
    let backend = PgBackend::connect_with(cfg.clone(), Some(&schema))
        .await
        .unwrap();
    backend.init_schema().await.unwrap();
    let claimed = match backend
        .claim_epoch_lease("postgres:detach", "owner", 15)
        .await
        .unwrap()
    {
        telex::model::EpochClaimResult::Claimed(claimed) => claimed,
        other => panic!("expected claim, got {other:?}"),
    };
    assert!(backend
        .release_epoch_lease_for_application_detach(
            "postgres:detach",
            "owner",
            claimed.lease_epoch,
            "postgres-application",
            "runtime",
            "bidirectional",
            "ApplicationDetach",
        )
        .await
        .unwrap());
    let intent = backend
        .application_detach_intent("postgres-application", "postgres:detach")
        .await
        .unwrap()
        .expect("application detach intent");
    assert_eq!(intent.runtime_id, "runtime");
    assert_eq!(
        backend
            .application_detach_intents("postgres-application")
            .await
            .unwrap(),
        vec![intent]
    );
    backend
        .clear_application_detach_intent("postgres-application", "postgres:detach")
        .await
        .unwrap();
    assert!(backend
        .application_detach_intent("postgres-application", "postgres:detach")
        .await
        .unwrap()
        .is_none());

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .unwrap();
}

#[tokio::test]
async fn postgres_concurrent_operation_duplicates_return_replay() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let Some(url) = pg_url_or_skip("postgres_concurrent_operation_duplicates_return_replay") else {
        return;
    };
    let cfg = pg_config(&url);
    let schema = sanitize_ident(&format!(
        "telex_app_operation_race_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .unwrap();
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .unwrap();
    let first = Arc::new(
        PgBackend::connect_with(cfg.clone(), Some(&schema))
            .await
            .unwrap(),
    );
    first.init_schema().await.unwrap();
    let second = Arc::new(
        PgBackend::connect_with(cfg.clone(), Some(&schema))
            .await
            .unwrap(),
    );
    let operation = Arc::new(NewApplicationOperation {
        logical_store_id: first.logical_store_id().await.unwrap(),
        application_responsibility: "postgres-race".into(),
        operation_id: "same-operation".into(),
        operation_kind: "send".into(),
        sender: "postgres:sender".into(),
        recipients_json: r#"["postgres:target"]"#.into(),
        payload_fingerprint: "a".repeat(64),
        retry_budget: 1,
        created_at_ms: now_ms(),
    });
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let first_task = {
        let backend = first.clone();
        let operation = operation.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            backend.begin_application_operation(&operation).await
        })
    };
    let second_task = {
        let backend = second.clone();
        let operation = operation.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            backend.begin_application_operation(&operation).await
        })
    };
    barrier.wait().await;
    let results = [first_task.await.unwrap(), second_task.await.unwrap()];
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Ok(ApplicationOperationBegin::Started(_))))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Ok(ApplicationOperationBegin::Replay(_))))
            .count(),
        1
    );

    let snapshot_operation = NewApplicationOperation {
        operation_id: "snapshot-operation".into(),
        ..operation.as_ref().clone()
    };
    let scope = ApplicationRecordScope {
        logical_store_id: snapshot_operation.logical_store_id.clone(),
        application_responsibility: snapshot_operation.application_responsibility.clone(),
    };
    let snapshot_barrier = Arc::new(tokio::sync::Barrier::new(2));
    let creator = {
        let backend = first.clone();
        let barrier = snapshot_barrier.clone();
        let operation = snapshot_operation.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            backend.begin_application_operation(&operation).await
        })
    };
    snapshot_barrier.wait().await;
    let snapshot = second
        .application_operation_snapshot(&scope, &snapshot_operation.operation_id)
        .await
        .unwrap();
    assert!(creator.await.unwrap().is_ok());
    assert_eq!(snapshot.retention_generation, 0);
    if let Some(record) = snapshot.operation {
        assert_eq!(record.operation_id, snapshot_operation.operation_id);
    }
    assert!(second
        .application_operation_snapshot(&scope, &snapshot_operation.operation_id)
        .await
        .unwrap()
        .operation
        .is_some());

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .unwrap();
}

#[tokio::test]
async fn postgres_operation_replay_racing_cleanup_stays_typed() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let Some(url) = pg_url_or_skip("postgres_operation_replay_racing_cleanup_stays_typed") else {
        return;
    };
    let cfg = pg_config(&url);
    let schema = sanitize_ident(&format!(
        "telex_app_replay_cleanup_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .unwrap();
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .unwrap();
    let replay_backend = Arc::new(
        PgBackend::connect_with(cfg.clone(), Some(&schema))
            .await
            .unwrap(),
    );
    replay_backend.init_schema().await.unwrap();
    let cleanup_backend = Arc::new(
        PgBackend::connect_with(cfg.clone(), Some(&schema))
            .await
            .unwrap(),
    );
    let logical_store_id = replay_backend.logical_store_id().await.unwrap();
    let scope = ApplicationRecordScope {
        logical_store_id: logical_store_id.clone(),
        application_responsibility: "postgres-race".into(),
    };

    for index in 0..32 {
        let operation = NewApplicationOperation {
            logical_store_id: logical_store_id.clone(),
            application_responsibility: scope.application_responsibility.clone(),
            operation_id: format!("cleanup-race-{index}"),
            operation_kind: "send".into(),
            sender: "postgres:sender".into(),
            recipients_json: r#"["postgres:target"]"#.into(),
            payload_fingerprint: "a".repeat(64),
            retry_budget: 1,
            created_at_ms: 1,
        };
        assert!(matches!(
            replay_backend
                .begin_application_operation(&operation)
                .await
                .unwrap(),
            ApplicationOperationBegin::Started(_)
        ));
        replay_backend
            .complete_application_operation(
                &operation.logical_store_id,
                &operation.application_responsibility,
                &operation.operation_id,
                "rejected",
                Some("{}"),
                None,
            )
            .await
            .unwrap();

        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let replay = {
            let backend = replay_backend.clone();
            let operation = operation.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                backend.begin_application_operation(&operation).await
            })
        };
        let cleanup = {
            let backend = cleanup_backend.clone();
            let scope = scope.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                backend
                    .cleanup_application_records(
                        &scope,
                        RetentionPolicy {
                            completed_before_ms: i64::MAX,
                            max_delete: 1,
                        },
                    )
                    .await
            })
        };
        barrier.wait().await;
        let replay = replay
            .await
            .unwrap()
            .expect("replay/cleanup race must remain typed");
        cleanup.await.unwrap().unwrap();
        assert!(matches!(
            replay,
            ApplicationOperationBegin::Replay(_) | ApplicationOperationBegin::Started(_)
        ));
    }

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .unwrap();
}

async fn wait_for_file(path: &std::path::Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

async fn wait_for_count(path: &std::path::Path, expected: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(path) {
            if text.trim().parse::<u32>().unwrap_or_default() >= expected {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[tokio::test]
async fn postgres_future_schema_version_fails_closed_before_mutation() {
    let Some(url) = pg_url_or_skip("postgres_future_schema_version_fails_closed_before_mutation")
    else {
        return;
    };
    let cfg = pg_config(&url);
    let schema = sanitize_ident(&format!(
        "telex_future_schema_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .expect("derived schema");
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("pre-test schema cleanup");
    admin_exec(
        &cfg,
        &format!(
            "CREATE SCHEMA {schema};
             CREATE TABLE {schema}.telex_schema_version(
                singleton integer NOT NULL DEFAULT 1 UNIQUE,
                version bigint NOT NULL
             );
             INSERT INTO {schema}.telex_schema_version(singleton, version) VALUES (1, 999);"
        ),
    )
    .await
    .expect("seed future schema version");

    let backend = PgBackend::connect_with(cfg.clone(), Some(&schema))
        .await
        .expect("connect future schema backend");
    let err = backend.init_schema().await.unwrap_err();
    assert!(
        err.to_string().contains("newer than supported"),
        "unexpected error: {err:#}"
    );

    let (client, connection) = cfg
        .connect(make_tls().expect("tls"))
        .await
        .expect("connect");
    let handle = tokio::spawn(async move {
        let _ = connection.await;
    });
    let addresses_exists: bool = client
        .query_one(
            &format!("SELECT to_regclass('{schema}.addresses') IS NOT NULL"),
            &[],
        )
        .await
        .expect("query addresses table")
        .get(0);
    assert!(
        !addresses_exists,
        "future schema gate must fail before creating ordinary telex tables"
    );
    drop(client);
    let _ = handle.await;

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("post-test schema cleanup");
}

#[tokio::test]
async fn postgres_profile_resolution_ambiguous_fails_closed() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prior_config = std::env::var_os("TELEX_CONFIG");
    let profile = BackendProfile {
        kind: "postgres".to_string(),
        path: None,
        url: Some("postgres://postgres:one@example.invalid/postgres".to_string()),
        auth: Some("password".to_string()),
        password_env: None,
        password_command: None,
        schema: Some("telex_ambiguous".to_string()),
        entra_cred: None,
        entra_scope: None,
    };
    let mut profile_two = profile.clone();
    profile_two.url = Some("postgres://postgres:two@example.invalid/postgres".to_string());
    let store_key = profiles::store_key(&profile, None);
    let mut backends = BTreeMap::new();
    backends.insert("pg-one".to_string(), profile);
    backends.insert("pg-two".to_string(), profile_two);
    let config_path = write_temp_config(
        "ambiguous",
        &ConfigFile {
            default: Some("pg-one".to_string()),
            backends,
        },
    );
    std::env::set_var("TELEX_CONFIG", &config_path);

    let daemon = TestDaemon::new("pg-ambiguous");
    let response = daemon.register(&store_key, "s1", "addr:a").await;
    assert!(
        matches!(response, Response::Error { ref code, ref message, .. }
            if code == proto::ERROR_UNSUPPORTED && message.contains("ambiguous Postgres backend profiles")),
        "ambiguous profile resolution must fail closed before connecting, got {response:?}"
    );

    let _ = std::fs::remove_dir_all(config_path.parent().unwrap());
    restore_env("TELEX_CONFIG", prior_config);
}

#[tokio::test]
async fn postgres_wake_on_cc_delivers_live_cc_without_replay() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(url) = pg_url_or_skip("postgres_wake_on_cc_delivers_live_cc_without_replay") else {
        return;
    };

    let prior_config = std::env::var_os("TELEX_CONFIG");
    let schema = sanitize_ident(&format!(
        "telex_daemon_pg_wake_cc_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .expect("derived schema");
    let cfg = pg_config(&url);
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("pre-test schema cleanup");

    let profile = BackendProfile {
        kind: "postgres".to_string(),
        path: None,
        url: Some(url.clone()),
        auth: Some("password".to_string()),
        password_env: std::env::var("TELEX_PG_PASSWORD")
            .ok()
            .filter(|pw| !pw.is_empty())
            .map(|_| "TELEX_PG_PASSWORD".to_string()),
        password_command: None,
        schema: Some(schema.clone()),
        entra_cred: None,
        entra_scope: None,
    };
    let store_key = profiles::store_key(&profile, None);
    let mut backends = BTreeMap::new();
    backends.insert("pg-wake-cc-test".to_string(), profile);
    let config_path = write_temp_config(
        "wake-cc",
        &ConfigFile {
            default: Some("pg-wake-cc-test".to_string()),
            backends,
        },
    );
    std::env::set_var("TELEX_CONFIG", &config_path);

    let daemon = TestDaemon::new("pg-wake-cc");
    registered_epoch(&daemon, &store_key, "primary", "addr:primary").await;
    registered_epoch(&daemon, &store_key, "observer", "addr:observer").await;
    let backend = daemon.backend(&store_key).await.expect("backend");
    let historical = insert_cc_message(&backend, "addr:primary", "addr:observer").await;

    let default_wait = daemon
        .wait(&store_key, "observer", "addr:observer", 1)
        .await;
    assert!(
        matches!(default_wait, Response::Timeout),
        "historical/default CC must remain pull-only, got {default_wait:?}"
    );

    let waiter = {
        let daemon = daemon.clone();
        let store_key = store_key.clone();
        tokio::spawn(async move {
            daemon
                .request(Request::Wait {
                    store_key,
                    session_id: "observer".to_string(),
                    address: "addr:observer".to_string(),
                    attention: None,
                    min_attention: None,
                    wake_on_cc: true,
                    timeout_ms: Some(1_000),
                    waiter_pid: Some(std::process::id()),
                    waiter_start_time: telex::session_watch::capture_process_start_time(
                        std::process::id(),
                    ),
                })
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(25)).await;
    let live = insert_cc_message(&backend, "addr:primary", "addr:observer").await;

    let delivered = waiter.await.expect("waiter");
    assert!(
        matches!(
            delivered,
            Response::Message {
                id,
                ref delivery_role,
                requires_disposition_for_current_recipient,
                ..
            } if id == live && delivery_role == "cc" && !requires_disposition_for_current_recipient
        ),
        "wake-on-cc should deliver live CC {live} and not historical {historical}, got {delivered:?}"
    );
    let rearm = daemon
        .request(Request::Wait {
            store_key: store_key.clone(),
            session_id: "observer".to_string(),
            address: "addr:observer".to_string(),
            attention: None,
            min_attention: None,
            wake_on_cc: true,
            timeout_ms: Some(1),
            waiter_pid: Some(std::process::id()),
            waiter_start_time: telex::session_watch::capture_process_start_time(std::process::id()),
        })
        .await;
    assert!(
        matches!(rearm, Response::Timeout),
        "wake-on-cc should not replay the delivered CC row, got {rearm:?}"
    );

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("post-test schema cleanup");
    let _ = std::fs::remove_dir_all(config_path.parent().unwrap());
    restore_env("TELEX_CONFIG", prior_config);
}

#[tokio::test]
async fn postgres_on_deliver_wake_on_cc_pushes_live_cc_without_replay() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(url) = pg_url_or_skip("postgres_on_deliver_wake_on_cc_pushes_live_cc_without_replay")
    else {
        return;
    };

    let prior_config = std::env::var_os("TELEX_CONFIG");
    let schema = sanitize_ident(&format!(
        "telex_daemon_pg_push_cc_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .expect("derived schema");
    let cfg = pg_config(&url);
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("pre-test schema cleanup");

    let profile = BackendProfile {
        kind: "postgres".to_string(),
        path: None,
        url: Some(url.clone()),
        auth: Some("password".to_string()),
        password_env: std::env::var("TELEX_PG_PASSWORD")
            .ok()
            .filter(|pw| !pw.is_empty())
            .map(|_| "TELEX_PG_PASSWORD".to_string()),
        password_command: None,
        schema: Some(schema.clone()),
        entra_cred: None,
        entra_scope: None,
    };
    let store_key = profiles::store_key(&profile, None);
    let mut backends = BTreeMap::new();
    backends.insert("pg-push-cc-test".to_string(), profile);
    let config_path = write_temp_config(
        "push-cc",
        &ConfigFile {
            default: Some("pg-push-cc-test".to_string()),
            backends,
        },
    );
    std::env::set_var("TELEX_CONFIG", &config_path);

    let daemon = TestDaemon::new("pg-push-cc");
    registered_epoch(&daemon, &store_key, "sender", "addr:sender").await;
    let output = std::env::temp_dir().join(format!(
        "telex-pg-push-cc-{}-{}.json",
        std::process::id(),
        now_ms()
    ));
    let _ = std::fs::remove_file(&output);

    let historical = daemon
        .request(send_request(
            &store_key,
            "sender",
            Some("addr:sender"),
            "addr:primary",
            Some("addr:observer"),
            "historical cc",
        ))
        .await;
    assert!(
        matches!(historical, Response::Sent { .. }),
        "historical send failed: {historical:?}"
    );

    let register = Request::Register {
        store_key: store_key.clone(),
        address: "addr:observer".to_string(),
        session_id: "observer".to_string(),
        occupant: "observer".to_string(),
        description: Some("observer push cc".to_string()),
        scope: None,
        tags: None,
        watch_pids: vec![WatchPidSpec::anchor(std::process::id())],
        replace_watch_pids: false,
        recovery: false,
        on_deliver: Some(record_stdin_argv(&output)),
        replace_on_deliver: false,
        on_deliver_wake_on_cc: true,
    };
    assert!(matches!(
        daemon.request(register).await,
        Response::Registered { .. }
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !output.exists(),
        "historical CC must not replay after push wake registration"
    );

    let live = daemon
        .request(send_request(
            &store_key,
            "sender",
            Some("addr:sender"),
            "addr:primary",
            Some("addr:observer"),
            "live cc",
        ))
        .await;
    let live_id = match live {
        Response::Sent { receipt } => receipt.id,
        other => panic!("live send failed: {other:?}"),
    };
    assert!(
        wait_for_file(&output, Duration::from_secs(10)).await,
        "live CC should be pushed through on-deliver"
    );
    let descriptor: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    assert_eq!(descriptor["message_id"], live_id);
    assert_eq!(descriptor["address"], "addr:observer");
    assert_eq!(descriptor["delivery_role"], "cc");
    assert_eq!(descriptor["primary_to"], "addr:primary");
    assert_eq!(
        descriptor["requires_disposition_for_current_recipient"],
        false
    );

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("post-test schema cleanup");
    let _ = std::fs::remove_dir_all(config_path.parent().unwrap());
    let _ = std::fs::remove_file(&output);
    restore_env("TELEX_CONFIG", prior_config);
}

#[tokio::test]
async fn postgres_on_deliver_failed_cc_retry_survives_later_accepted_cc() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(url) =
        pg_url_or_skip("postgres_on_deliver_failed_cc_retry_survives_later_accepted_cc")
    else {
        return;
    };

    let prior_config = std::env::var_os("TELEX_CONFIG");
    let schema = sanitize_ident(&format!(
        "telex_daemon_pg_push_cc_retry_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .expect("derived schema");
    let cfg = pg_config(&url);
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("pre-test schema cleanup");

    let profile = BackendProfile {
        kind: "postgres".to_string(),
        path: None,
        url: Some(url.clone()),
        auth: Some("password".to_string()),
        password_env: std::env::var("TELEX_PG_PASSWORD")
            .ok()
            .filter(|pw| !pw.is_empty())
            .map(|_| "TELEX_PG_PASSWORD".to_string()),
        password_command: None,
        schema: Some(schema.clone()),
        entra_cred: None,
        entra_scope: None,
    };
    let store_key = profiles::store_key(&profile, None);
    let mut backends = BTreeMap::new();
    backends.insert("pg-push-cc-retry-test".to_string(), profile);
    let config_path = write_temp_config(
        "push-cc-retry",
        &ConfigFile {
            default: Some("pg-push-cc-retry-test".to_string()),
            backends,
        },
    );
    std::env::set_var("TELEX_CONFIG", &config_path);

    let daemon = TestDaemon::new("pg-push-cc-retry");
    registered_epoch(&daemon, &store_key, "sender", "addr:sender").await;
    let output_root = std::env::temp_dir().join(format!(
        "telex-pg-push-cc-retry-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let _ = std::fs::remove_dir_all(&output_root);

    let register = Request::Register {
        store_key: store_key.clone(),
        address: "addr:observer".to_string(),
        session_id: "observer".to_string(),
        occupant: "observer".to_string(),
        description: Some("observer push cc retry".to_string()),
        scope: None,
        tags: None,
        watch_pids: vec![WatchPidSpec::anchor(std::process::id())],
        replace_watch_pids: false,
        recovery: false,
        on_deliver: Some(fail_first_then_record_argv(&output_root)),
        replace_on_deliver: false,
        on_deliver_wake_on_cc: true,
    };
    assert!(matches!(
        daemon.request(register).await,
        Response::Registered { .. }
    ));

    let first = daemon
        .request(send_request(
            &store_key,
            "sender",
            Some("addr:sender"),
            "addr:primary",
            Some("addr:observer"),
            "first cc",
        ))
        .await;
    let first_id = match first {
        Response::Sent { receipt } => receipt.id,
        other => panic!("first live CC send failed: {other:?}"),
    };
    let first_count = output_root.join("first.count");
    assert!(
        wait_for_count(&first_count, 1, Duration::from_secs(10)).await,
        "first CC should be attempted once and fail transiently"
    );
    let mut rewound = false;
    for _ in 0..100 {
        if daemon.rewind_on_deliver_attempt(
            &store_key,
            "observer",
            "addr:observer",
            first_id,
            Duration::from_secs(60),
        ) {
            rewound = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        rewound,
        "failed first CC attempt should be recorded in push bookkeeping"
    );

    let second = daemon
        .request(send_request(
            &store_key,
            "sender",
            Some("addr:sender"),
            "addr:primary",
            Some("addr:observer"),
            "second cc",
        ))
        .await;
    let second_id = match second {
        Response::Sent { receipt } => receipt.id,
        other => panic!("second live CC send failed: {other:?}"),
    };
    let second_path = output_root.join("second.json");
    assert!(
        wait_for_file(&second_path, Duration::from_secs(10)).await,
        "second CC should be accepted by the handler"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    daemon.heartbeat_once().await;
    assert!(
        wait_for_count(&first_count, 2, Duration::from_secs(10)).await,
        "failed first CC should remain retryable after later CC succeeds"
    );
    let first_retry_path = output_root.join("first-retry.json");
    assert!(
        wait_for_file(&first_retry_path, Duration::from_secs(10)).await,
        "first CC retry descriptor should be written after the retry count advances"
    );
    let retry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(first_retry_path).unwrap()).unwrap();
    assert_eq!(retry["message_id"], first_id);
    assert_eq!(retry["delivery_role"], "cc");
    let second_descriptor: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(second_path).unwrap()).unwrap();
    assert_eq!(second_descriptor["message_id"], second_id);
    assert_eq!(second_descriptor["delivery_role"], "cc");

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("post-test schema cleanup");
    let _ = std::fs::remove_dir_all(config_path.parent().unwrap());
    let _ = std::fs::remove_dir_all(&output_root);
    restore_env("TELEX_CONFIG", prior_config);
}

#[tokio::test]
async fn postgres_competing_daemon_epoch_self_demotes_without_double_delivery() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(url) =
        pg_url_or_skip("postgres_competing_daemon_epoch_self_demotes_without_double_delivery")
    else {
        return;
    };

    let prior_config = std::env::var_os("TELEX_CONFIG");
    let prior_liveness = std::env::var_os("TELEX_LIVENESS_WINDOW_SECS");
    std::env::set_var("TELEX_LIVENESS_WINDOW_SECS", "0");

    let schema = sanitize_ident(&format!(
        "telex_daemon_pg_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .expect("derived schema");
    let cfg = pg_config(&url);
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("pre-test schema cleanup");

    let mut profile = BackendProfile {
        kind: "postgres".to_string(),
        path: None,
        url: Some(url.clone()),
        auth: Some("password".to_string()),
        password_env: std::env::var("TELEX_PG_PASSWORD")
            .ok()
            .filter(|pw| !pw.is_empty())
            .map(|_| "TELEX_PG_PASSWORD".to_string()),
        password_command: None,
        schema: Some(schema.clone()),
        entra_cred: None,
        entra_scope: None,
    };
    if profile.password_env.is_none() {
        profile.auth = Some("password".to_string());
    }
    let store_key = profiles::store_key(&profile, None);
    let mut backends = BTreeMap::new();
    backends.insert("pg-daemon-test".to_string(), profile);
    let config = ConfigFile {
        default: Some("pg-daemon-test".to_string()),
        backends,
    };
    let root = std::env::temp_dir().join(format!(
        "telex-daemon-pg-config-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&root).expect("create temp config dir");
    let config_path = root.join("config.toml");
    std::fs::write(
        &config_path,
        toml::to_string_pretty(&config).expect("serialize config"),
    )
    .expect("write temp config");
    std::env::set_var("TELEX_CONFIG", &config_path);

    let first = TestDaemon::new("pg-compete-first");
    let second = TestDaemon::new("pg-compete-second");
    let (epoch1, _) = registered_epoch(&first, &store_key, "s1", "addr:a").await;
    let backend = first.backend(&store_key).await.expect("backend");
    let message_id = insert_message(&backend, "addr:a").await;

    tokio::time::sleep(Duration::from_millis(20)).await;
    let (epoch2, _) = registered_epoch(&second, &store_key, "s2", "addr:a").await;
    assert!(epoch2 > epoch1, "successor must claim a higher epoch");

    let stale_wait = first.wait(&store_key, "s1", "addr:a", 1_000).await;
    assert!(
        matches!(stale_wait, Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH || code == proto::ERROR_NOT_OWNER),
        "stale owner must self-demote before emitting, got {stale_wait:?}"
    );
    assert!(first.status().await.members.is_empty());

    let successor_wait = second.wait(&store_key, "s2", "addr:a", 1_000).await;
    assert!(
        matches!(successor_wait, Response::Message { id, .. } if id == message_id),
        "successor should deliver the pending message once, got {successor_wait:?}"
    );
    match second.ack(&store_key, "s2", "addr:a", message_id).await {
        Response::Ack {
            delivery_outcome, ..
        } => assert_eq!(delivery_outcome, Some(DeliveryOutcome::Marked)),
        other => panic!("expected successor Ack, got {other:?}"),
    }

    let after_ack = second.wait(&store_key, "s2", "addr:a", 1).await;
    assert!(
        matches!(after_ack, Response::Timeout),
        "Ack must consume the delivery for the current owner, got {after_ack:?}"
    );

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("post-test schema cleanup");
    let _ = std::fs::remove_dir_all(&root);
    restore_env("TELEX_CONFIG", prior_config);
    restore_env("TELEX_LIVENESS_WINDOW_SECS", prior_liveness);
}

#[tokio::test]
async fn postgres_listen_notify_wakes_blocked_waiter() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(url) = pg_url_or_skip("postgres_listen_notify_wakes_blocked_waiter") else {
        return;
    };

    let prior_config = std::env::var_os("TELEX_CONFIG");
    let schema = sanitize_ident(&format!(
        "telex_daemon_pg_notify_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .expect("derived schema");
    let cfg = pg_config(&url);
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("pre-test schema cleanup");

    let profile = BackendProfile {
        kind: "postgres".to_string(),
        path: None,
        url: Some(url.clone()),
        auth: Some("password".to_string()),
        password_env: std::env::var("TELEX_PG_PASSWORD")
            .ok()
            .filter(|pw| !pw.is_empty())
            .map(|_| "TELEX_PG_PASSWORD".to_string()),
        password_command: None,
        schema: Some(schema.clone()),
        entra_cred: None,
        entra_scope: None,
    };
    let store_key = profiles::store_key(&profile, None);
    let mut backends = BTreeMap::new();
    backends.insert("pg-notify-test".to_string(), profile);
    let config = ConfigFile {
        default: Some("pg-notify-test".to_string()),
        backends,
    };
    let root = std::env::temp_dir().join(format!(
        "telex-daemon-pg-notify-config-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&root).expect("create temp config dir");
    let config_path = root.join("config.toml");
    std::fs::write(
        &config_path,
        toml::to_string_pretty(&config).expect("serialize config"),
    )
    .expect("write temp config");
    std::env::set_var("TELEX_CONFIG", &config_path);

    let daemon = TestDaemon::new("pg-notify");
    registered_epoch(&daemon, &store_key, "receiver", "addr:receiver").await;
    registered_epoch(&daemon, &store_key, "sender", "addr:sender").await;
    tokio::time::sleep(Duration::from_millis(250)).await;

    let waiter = {
        let daemon = daemon.clone();
        let store_key = store_key.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            let response = daemon
                .wait(&store_key, "receiver", "addr:receiver", 1_000)
                .await;
            (start.elapsed(), response)
        })
    };
    tokio::time::sleep(Duration::from_millis(25)).await;
    let sent = daemon
        .request(send_request(
            &store_key,
            "sender",
            Some("addr:sender"),
            "addr:receiver",
            None,
            "notify wake",
        ))
        .await;
    assert!(
        matches!(sent, Response::Sent { .. }),
        "send failed: {sent:?}"
    );
    let (elapsed, response) = waiter.await.expect("waiter task");
    let delivery_latency_ms = match &response {
        Response::Message {
            body,
            sent_at_ms,
            buffered_at_ms,
            ..
        } if body == "notify wake" => (*buffered_at_ms).saturating_sub(*sent_at_ms),
        _ => panic!("waiter should receive message, got {response:?}"),
    };
    assert!(
        delivery_latency_ms < 100,
        "LISTEN/NOTIFY should wake before the 100ms polling fallback; waiter_elapsed={elapsed:?}, delivery_latency_ms={delivery_latency_ms}"
    );

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("post-test schema cleanup");
    let _ = std::fs::remove_dir_all(&root);
    restore_env("TELEX_CONFIG", prior_config);
}

#[tokio::test]
async fn postgres_listener_degradation_surfaces_recent_error() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(url) = pg_url_or_skip("postgres_listener_degradation_surfaces_recent_error") else {
        return;
    };

    let prior_config = std::env::var_os("TELEX_CONFIG");
    let schema = sanitize_ident(&format!(
        "telex_daemon_pg_degraded_{}_{}",
        std::process::id(),
        now_ms()
    ))
    .expect("derived schema");
    let cfg = pg_config(&url);
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("pre-test schema cleanup");

    let profile = BackendProfile {
        kind: "postgres".to_string(),
        path: None,
        url: Some(url.clone()),
        auth: Some("password".to_string()),
        password_env: std::env::var("TELEX_PG_PASSWORD")
            .ok()
            .filter(|pw| !pw.is_empty())
            .map(|_| "TELEX_PG_PASSWORD".to_string()),
        password_command: None,
        schema: Some(schema.clone()),
        entra_cred: None,
        entra_scope: None,
    };
    let store_key = profiles::store_key(&profile, None);
    let mut backends = BTreeMap::new();
    backends.insert("pg-degraded-test".to_string(), profile);
    let config_path = write_temp_config(
        "degraded",
        &ConfigFile {
            default: Some("pg-degraded-test".to_string()),
            backends,
        },
    );
    std::env::set_var("TELEX_CONFIG", &config_path);

    let daemon = TestDaemon::new("pg-degraded");
    registered_epoch(&daemon, &store_key, "receiver", "addr:receiver").await;
    tokio::time::sleep(Duration::from_millis(250)).await;

    admin_exec(
        &cfg,
        "SELECT pg_terminate_backend(pid)
         FROM pg_stat_activity
         WHERE pid <> pg_backend_pid()
           AND query LIKE 'LISTEN telex_messages_%'",
    )
    .await
    .expect("terminate listener backend");

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let status = daemon.status().await;
        if status
            .recent_errors
            .iter()
            .any(|err| err.kind == "NotifyDegraded" && err.message.contains("LISTEN loop"))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "expected NotifyDegraded recent error after listener termination, got {:?}",
            status.recent_errors
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("post-test schema cleanup");
    let _ = std::fs::remove_dir_all(config_path.parent().unwrap());
    restore_env("TELEX_CONFIG", prior_config);
}

#[tokio::test]
async fn postgres_reset_does_not_abort_pull_or_unregister_push_station() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(url) = pg_url_or_skip("postgres_reset_does_not_abort_pull_or_unregister_push_station")
    else {
        return;
    };

    let prior_config = std::env::var_os("TELEX_CONFIG");
    let unique = format!("{}_{}", std::process::id(), now_ms());
    let schema =
        sanitize_ident(&format!("telex_daemon_pg_reset_{unique}")).expect("derived schema");
    let application_name =
        sanitize_ident(&format!("telex_wait_reset_{unique}")).expect("application name");
    let cfg = pg_config(&url);
    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("pre-test schema cleanup");

    let profile = BackendProfile {
        kind: "postgres".to_string(),
        path: None,
        url: Some(pg_url_with_application_name(&url, &application_name)),
        auth: Some("password".to_string()),
        password_env: std::env::var("TELEX_PG_PASSWORD")
            .ok()
            .filter(|pw| !pw.is_empty())
            .map(|_| "TELEX_PG_PASSWORD".to_string()),
        password_command: None,
        schema: Some(schema.clone()),
        entra_cred: None,
        entra_scope: None,
    };
    let store_key = profiles::store_key(&profile, None);
    let mut backends = BTreeMap::new();
    backends.insert("pg-reset-test".to_string(), profile);
    let config_path = write_temp_config(
        "reset",
        &ConfigFile {
            default: Some("pg-reset-test".to_string()),
            backends,
        },
    );
    std::env::set_var("TELEX_CONFIG", &config_path);

    let daemon = TestDaemon::new("pg-reset");
    registered_epoch(&daemon, &store_key, "pull", "addr:pull").await;
    registered_epoch(&daemon, &store_key, "sender", "addr:sender").await;
    let push_output = config_path.parent().unwrap().join("push.json");
    let push_register = Request::Register {
        store_key: store_key.clone(),
        address: "addr:push".to_string(),
        session_id: "push".to_string(),
        occupant: "push".to_string(),
        description: None,
        scope: None,
        tags: None,
        watch_pids: vec![WatchPidSpec::anchor(std::process::id())],
        replace_watch_pids: false,
        recovery: false,
        on_deliver: Some(record_stdin_argv(&push_output)),
        replace_on_deliver: false,
        on_deliver_wake_on_cc: false,
    };
    assert!(matches!(
        daemon.request(push_register).await,
        Response::Registered { .. }
    ));
    tokio::time::sleep(Duration::from_millis(250)).await;

    admin_exec(
        &cfg,
        &format!(
            "SET search_path TO {schema}, public;
             CREATE SEQUENCE wait_fault_seq;
             CREATE FUNCTION fail_first_pull_delivery() RETURNS trigger AS $$
             BEGIN
               IF NEW.recipient = 'addr:pull'
                  AND nextval('{schema}.wait_fault_seq') = 1 THEN
                 RAISE EXCEPTION 'injected transient shutdown' USING ERRCODE = '57P01';
               END IF;
               RETURN NEW;
             END
             $$ LANGUAGE plpgsql;
             CREATE TRIGGER fail_first_pull_delivery
               BEFORE INSERT ON deliveries
               FOR EACH ROW EXECUTE FUNCTION fail_first_pull_delivery();"
        ),
    )
    .await
    .expect("install one-shot wait fetch fault");

    let waiter = {
        let daemon = daemon.clone();
        let store_key = store_key.clone();
        tokio::spawn(async move { daemon.wait(&store_key, "pull", "addr:pull", 5_000).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    admin_exec(
        &cfg,
        &format!(
            "SELECT pg_terminate_backend(pid)
             FROM pg_stat_activity
             WHERE pid <> pg_backend_pid()
               AND application_name = '{application_name}'"
        ),
    )
    .await
    .expect("terminate daemon postgres connections");

    let notify_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if daemon
            .status()
            .await
            .recent_errors
            .iter()
            .any(|error| error.kind == "NotifyDegraded")
        {
            break;
        }
        assert!(
            Instant::now() < notify_deadline,
            "expected LISTEN connection degradation after reset"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let sent = daemon
        .request(send_request(
            &store_key,
            "sender",
            Some("addr:sender"),
            "addr:pull",
            None,
            "pull after reset",
        ))
        .await;
    assert!(
        matches!(sent, Response::Sent { .. }),
        "send failed: {sent:?}"
    );
    assert!(matches!(
        waiter.await.expect("waiter task"),
        Response::Message { ref body, .. } if body == "pull after reset"
    ));

    let pushed = daemon
        .request(send_request(
            &store_key,
            "sender",
            Some("addr:sender"),
            "addr:push",
            None,
            "push after reset",
        ))
        .await;
    assert!(
        matches!(pushed, Response::Sent { .. }),
        "send failed: {pushed:?}"
    );
    let push_deadline = Instant::now() + Duration::from_secs(3);
    while !push_output.exists() {
        assert!(
            Instant::now() < push_deadline,
            "push station did not deliver after reset"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let status = daemon.status().await;
    assert!(status
        .recent_errors
        .iter()
        .any(|error| error.kind == "BackendDegraded"));
    assert!(status.members.iter().any(|member| {
        member.session_id == "push"
            && member.address == "addr:push"
            && member.push_registered
            && !member.idle
    }));

    admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .expect("post-test schema cleanup");
    let _ = std::fs::remove_dir_all(config_path.parent().unwrap());
    restore_env("TELEX_CONFIG", prior_config);
}
