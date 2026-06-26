#![cfg(all(feature = "postgres", feature = "sqlite"))]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use telex::backend::postgres::{make_tls, sanitize_ident};
use telex::backend::Backend;
use telex::daemon::test_support::{registered_epoch, TestDaemon};
use telex::daemon_ipc::{self as proto, Response};
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
