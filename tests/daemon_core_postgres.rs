#![cfg(all(feature = "postgres", feature = "sqlite"))]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use telex::backend::postgres::{make_tls, sanitize_ident, PgBackend};
use telex::backend::Backend;
use telex::daemon::test_support::{registered_epoch, send_request, TestDaemon};
use telex::daemon_ipc::{self as proto, Request, Response, WatchPidSpec};
use telex::model::{now_ms, Attention, DeliveryOutcome, NewMessage};
use telex::profiles::{self, BackendProfile, ConfigFile};

static ENV_LOCK: Mutex<()> = Mutex::new(());

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
        recovery: false,
        on_deliver: Some(record_stdin_argv(&output)),
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
        recovery: false,
        on_deliver: Some(fail_first_then_record_argv(&output_root)),
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
    assert!(
        matches!(response, Response::Message { ref body, .. } if body == "notify wake"),
        "waiter should receive message, got {response:?}"
    );
    assert!(
        elapsed < Duration::from_millis(90),
        "LISTEN/NOTIFY should wake before the 100ms polling fallback; elapsed={elapsed:?}"
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
