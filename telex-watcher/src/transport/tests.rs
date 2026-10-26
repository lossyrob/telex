use super::*;
use crate::ownership::Ownership;
use serde_json::{json, Value};
use std::path::PathBuf;

pub(super) struct Fixture {
    root: PathBuf,
    pub(super) registry: PathBuf,
    ownership: Option<Ownership>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let root = std::env::current_dir()
            .unwrap()
            .join(format!(".transport-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let registry = root.join("registry.sqlite");
        let ownership = Some(Ownership::acquire(&registry).unwrap());
        Self {
            root,
            registry,
            ownership,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.ownership.take();
        let removed = std::fs::remove_dir_all(&self.root);
        // A second panic while unwinding aborts the whole test harness
        // (0xc0000409 on Windows) and discards the original failure.
        if !std::thread::panicking() {
            removed.unwrap();
        }
    }
}

fn testing_limits() -> Limits {
    Limits {
        read: Duration::from_millis(200),
        write: Duration::from_millis(200),
        execute: Duration::from_millis(200),
        response: Duration::from_secs(2),
    }
}

async fn receive_response<S: AsyncRead + Unpin>(connection: &mut S) -> ResponseFrame {
    let bytes = timeout(
        Duration::from_secs(3),
        read_frame(connection, MAX_RESPONSE_BYTES),
    )
    .await
    .expect("bounded response deadline")
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn connect_native(registry: &Path) -> impl AsyncRead + AsyncWrite + Unpin {
    platform::connect(&endpoint_key(registry).unwrap())
        .await
        .unwrap()
}

#[test]
fn commands_round_trip_without_adding_operations() {
    let cwd = std::env::current_dir().unwrap();
    let commands = vec![
        Command::Register {
            json: vec![0, 255],
            working_directory: cwd.clone(),
        },
        Command::List {
            include_removed: true,
            include_unsupported: false,
        },
        Command::Show { id: "w".into() },
        Command::Pause { id: "w".into() },
        Command::Resume { id: "w".into() },
        Command::Update {
            id: "w".into(),
            json: vec![255],
            working_directory: cwd,
            state: Some(Value::Null),
        },
        Command::ReconcileNow { id: "w".into() },
        Command::GrantReconciliation {
            id: "w".into(),
            attempts: 2,
            seconds: 3,
        },
        Command::GrantSendRetry {
            id: "w".into(),
            attempts: 2,
            seconds: 3,
        },
        Command::CloseNotAccepted { id: "w".into() },
        Command::Remove { id: "w".into() },
        Command::Health,
        Command::WatchHealth { id: "w".into() },
        Command::Attempts {
            id: "w".into(),
            limit: 4,
        },
        Command::Events {
            id: "w".into(),
            limit: 4,
        },
        Command::Pending { id: "w".into() },
        Command::ReceiptEvidence {
            operation_id: "op".into(),
            limit: 4,
        },
        Command::RemovedEvidence {
            id: "w".into(),
            limit: 4,
        },
        Command::Shutdown,
    ];
    for command in commands {
        let original = serde_json::to_value(command).unwrap();
        let decoded: Command = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), original);
    }
}

#[test]
fn omitted_state_and_explicit_null_remain_distinct_and_unknown_fields_fail() {
    for state in [None, Some(Value::Null), Some(json!({"sequence": 1}))] {
        let command = Command::Update {
            id: "state".into(),
            json: b"{}".to_vec(),
            working_directory: std::env::current_dir().unwrap(),
            state: state.clone(),
        };
        let encoded = serde_json::to_value(&command).unwrap();
        assert_eq!(encoded["value"].get("state"), state.as_ref());
        let Command::Update { state: decoded, .. } =
            serde_json::from_value(encoded.clone()).unwrap()
        else {
            panic!("wrong command")
        };
        assert_eq!(decoded, state);
        let mut invalid = encoded;
        invalid["value"]["surprise"] = json!(true);
        assert!(serde_json::from_value::<Command>(invalid).is_err());
    }
    assert!(serde_json::from_value::<RequestFrame>(json!({
        "version": 1, "command": {"type": "health"}, "surprise": true
    }))
    .is_err());
    assert!(serde_json::from_slice::<RequestFrame>(
        br#"{"version":1,"version":1,"command":{"type":"health"}}"#
    )
    .is_err());
}

#[test]
fn byte_array_transport_admits_two_mib_and_preserves_duplicate_keys() {
    let bytes = vec![255; 2 * 1024 * 1024];
    let frame = RequestFrame {
        version: PROTOCOL_VERSION,
        filters: None,
        command: Command::Register {
            json: bytes.clone(),
            working_directory: std::env::current_dir().unwrap(),
        },
    };
    let encoded = encode(&frame, MAX_REQUEST_BYTES).unwrap();
    let decoded: RequestFrame = serde_json::from_slice(&encoded).unwrap();
    let Command::Register { json, .. } = decoded.command else {
        panic!("wrong command")
    };
    assert_eq!(json, bytes);
    assert!(encode(&frame, 10).is_err());
}

#[test]
fn typed_response_decoding_does_not_apply_detector_state_numeric_restrictions() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/evidence-v2.json")).unwrap();
    let mut health: crate::evidence::RuntimeHealth =
        serde_json::from_value(corpus["runtime"].clone()).unwrap();
    health.registry_revision = u64::MAX;
    let bytes = encode(
        &ResponseFrame {
            version: PROTOCOL_VERSION,
            result: Ok(Envelope::from_response(
                "health",
                &Response::Health(health),
                None,
                0,
            )),
        },
        MAX_RESPONSE_BYTES,
    )
    .unwrap();
    let decoded: ResponseFrame = serde_json::from_slice(&bytes).unwrap();
    let result = decoded.result.unwrap().result.unwrap();
    assert_eq!(result.r#type, "health");
    assert_eq!(result.value["registryRevision"], u64::MAX);
}

#[tokio::test]
async fn projected_wire_schema_and_read_selectors_are_explicit() {
    let (mut writer, mut reader) = tokio::io::duplex(1024);
    write_frame(
        &mut writer,
        br#"{"version":1,"command":{"type":"list","value":{"includeRemoved":false,"includeUnsupported":false}},"filters":{"paused":true}}"#,
    )
    .await
    .unwrap();
    let frame = read_request(&mut reader).await.unwrap();
    assert!(matches!(
        frame.command,
        Command::List {
            include_removed: false,
            include_unsupported: false,
        }
    ));
    assert!(frame.filters.unwrap().paused);
    let response = ResponseFrame {
        version: PROTOCOL_VERSION,
        result: Ok(Envelope::from_response(
            "list",
            &Response::Watches(vec![]),
            None,
            0,
        )),
    };
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "version":1,
            "result":{"Ok":{
                "schemaVersion":1,
                "command":"list",
                "ok":true,
                "result":{"type":"watches","value":[]}
            }}
        })
    );
}

#[tokio::test]
async fn native_same_user_round_trip_preserves_state_and_registration_bytes() {
    let fixture = Fixture::new();
    let (management, mut requests) = LocalManagement::channel();
    let server = Server::bind(&fixture.registry, management).unwrap();
    let server = tokio::spawn(server.run());
    let registration = br#"{"version":2,"version":2}"#.to_vec();
    for state in [None, Some(Value::Null), Some(json!({"n": 1}))] {
        let registry = fixture.registry.clone();
        let expected = state.clone();
        let registration = registration.clone();
        let expected_registration = registration.clone();
        let client = tokio::spawn(async move {
            execute(
                &registry,
                Command::Update {
                    id: "state".into(),
                    json: registration,
                    working_directory: std::env::current_dir().unwrap(),
                    state,
                },
                None,
            )
            .await
        });
        let request = timeout(Duration::from_secs(3), requests.recv())
            .await
            .unwrap()
            .unwrap();
        let Command::Update {
            state,
            json,
            working_directory,
            ..
        } = request.command
        else {
            panic!("wrong command")
        };
        assert_eq!(state, expected);
        assert_eq!(json, expected_registration);
        assert!(crate::protocol_v2::parse_registration(&json, &working_directory).is_err());
        request.reply.send(Ok(Response::Stopping)).unwrap();
        assert_eq!(
            client.await.unwrap().unwrap().result.unwrap().r#type,
            "stopping"
        );
    }
    // The client looked up metadata, never took the still-held registry lock.
    assert!(Ownership::acquire(&fixture.registry).is_err());
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn native_alias_and_hard_link_reach_the_same_owner() {
    let fixture = Fixture::new();
    let alias_dir = fixture.root.join("aliases");
    std::fs::create_dir(&alias_dir).unwrap();
    let hard_link = alias_dir.join("hard-link.sqlite");
    std::fs::hard_link(&fixture.registry, &hard_link).unwrap();
    let alias = alias_dir.join("..").join("registry.sqlite");
    let mut paths = vec![alias, hard_link];
    #[cfg(unix)]
    {
        let symlink = alias_dir.join("symbolic-link.sqlite");
        std::os::unix::fs::symlink(&fixture.registry, &symlink).unwrap();
        paths.push(symlink);
    }
    #[cfg(windows)]
    let _ = &mut paths;
    let (management, mut requests) = LocalManagement::channel();
    let server = tokio::spawn(Server::bind(&fixture.registry, management).unwrap().run());
    for path in paths {
        assert_eq!(
            endpoint_key(&path).unwrap(),
            endpoint_key(&fixture.registry).unwrap()
        );
        let client = tokio::spawn(async move { execute(&path, Command::Health, None).await });
        let request = timeout(Duration::from_secs(3), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(request.command, Command::Health));
        request.reply.send(Ok(Response::Watch(None))).unwrap();
        let envelope = client.await.unwrap().unwrap();
        assert_eq!(envelope.result.unwrap().value, Value::Null);
        assert_eq!(envelope.error.unwrap().code, "not-found");
    }
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn native_missing_owner_is_typed_bounded_and_never_creates_registry() {
    let fixture = Fixture::new();
    for registry in [&fixture.registry, &fixture.root.join("missing.sqlite")] {
        let error = timeout(
            Duration::from_secs(3),
            execute(registry, Command::Health, None),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(error.code(), "runtime-not-running");
        let value = serde_json::to_value(&error).unwrap();
        assert_eq!(value["code"], "runtime-not-running");
        assert_eq!(
            serde_json::from_value::<TransportError>(value).unwrap(),
            error
        );
    }
    assert!(!fixture.root.join("missing.sqlite").exists());
}

#[tokio::test]
async fn native_oversized_malformed_versioned_and_slow_clients_are_bounded() {
    let fixture = Fixture::new();
    let (management, mut requests) = LocalManagement::channel();
    let mut server = Server::bind(&fixture.registry, management).unwrap();
    server.limits = testing_limits();
    let server = tokio::spawn(server.run());

    let mut oversized = connect_native(&fixture.registry).await;
    oversized
        .write_u32((MAX_REQUEST_BYTES + 1) as u32)
        .await
        .unwrap();
    assert_eq!(
        receive_response(&mut oversized)
            .await
            .result
            .unwrap_err()
            .code(),
        "invalid-request"
    );
    for frame in [
        b"not json".as_slice(),
        br#"{"version":2,"command":{"type":"health"}}"#,
        br#"{"version":1,"command":{"type":"health"},"x":1}"#,
        br#"{"version":1,"command":{"type":"pause","value":{"id":"w","x":1}}}"#,
        br#"{"version":1,"command":{"type":"health"},"filters":{"active":true}}"#,
        br#"{"version":1,"command":{"type":"pause","value":{"id":"w"}},"filters":{}}"#,
        br#"{"version":1,"command":{"type":"show","value":{"id":"w"}},"filters":{"unknown":true}}"#,
        br#"{"version":1,"command":{"type":"show","value":{"id":"w"}},"filters":{"active":"true"}}"#,
        br#"{"version":1,"command":{"type":"show","value":{"id":"w"}},"filters":{"active":true,"active":false}}"#,
        br#"{"version":1,"command":{"type":"list","value":{"includeRemoved":false,"includeUnsupported":false}},"filters":{"all":true}}"#,
        br#"{"version":1,"command":{"type":"list","value":{"includeRemoved":false,"includeUnsupported":false}},"filters":{"unsupportedV1":true}}"#,
        br#"{"version":1,"command":{"type":"update","value":{"id":"w","json":[],"workingDirectory":"local","state":{"nested":{"n":1,"n":2}}}}}"#,
        br#"{"version":1,"command":{"type":"update","value":{"id":"w","json":[],"workingDirectory":"local","state":{"n":9007199254740993}}}}"#,
        br#"{"version":1,"command":{"type":"update","value":{"id":"w","json":[],"workingDirectory":"local","state":{"n":1e-999}}}}"#,
        br#"{"version":1,"command":{"type":"update","value":{"id":"w","json":[],"workingDirectory":"local","state":{"text":"\ud800"}}}}"#,
    ] {
        let mut connection = connect_native(&fixture.registry).await;
        write_frame(&mut connection, frame).await.unwrap();
        assert_eq!(
            receive_response(&mut connection)
                .await
                .result
                .unwrap_err()
                .code(),
            "invalid-request"
        );
    }
    let mut slow = connect_native(&fixture.registry).await;
    slow.write_u32(1024).await.unwrap();
    slow.write_all(b"{").await.unwrap();
    assert_eq!(
        receive_response(&mut slow).await.result.unwrap_err().code(),
        "timeout"
    );
    assert!(requests.try_recv().is_err());
    drop((oversized, slow));
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn native_concurrency_limit_returns_busy_without_admitting_another_command() {
    let fixture = Fixture::new();
    let (management, mut requests) = LocalManagement::channel();
    let server = tokio::spawn(Server::bind(&fixture.registry, management).unwrap().run());
    let mut clients = Vec::new();
    let mut admitted = Vec::new();
    for _ in 0..MAX_CLIENTS {
        let registry = fixture.registry.clone();
        clients.push(tokio::spawn(async move {
            execute(&registry, Command::Health, None).await
        }));
        admitted.push(
            timeout(Duration::from_secs(3), requests.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(
        execute(&fixture.registry, Command::Health, None)
            .await
            .unwrap_err()
            .code(),
        "busy"
    );
    assert!(requests.try_recv().is_err());
    for request in admitted {
        request.reply.send(Ok(Response::Stopping)).unwrap();
    }
    for client in clients {
        assert_eq!(
            client.await.unwrap().unwrap().result.unwrap().r#type,
            "stopping"
        );
    }
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn native_admitted_command_survives_disconnect_and_timeout() {
    let fixture = Fixture::new();
    let (management, mut requests) = LocalManagement::channel();
    let mut server = Server::bind(&fixture.registry, management).unwrap();
    server.limits = testing_limits();
    let server = tokio::spawn(server.run());
    let mut disconnected = connect_native(&fixture.registry).await;
    write_frame(
        &mut disconnected,
        &encode(
            &RequestFrame {
                version: PROTOCOL_VERSION,
                filters: None,
                command: Command::Remove {
                    id: "admitted".into(),
                },
            },
            MAX_REQUEST_BYTES,
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let request = timeout(Duration::from_secs(3), requests.recv())
        .await
        .unwrap()
        .unwrap();
    drop(disconnected);
    assert!(matches!(request.command, Command::Remove { ref id } if id == "admitted"));
    assert!(request.reply.send(Ok(Response::Stopping)).is_ok());

    let registry = fixture.registry.clone();
    let client = tokio::spawn(async move {
        execute(
            &registry,
            Command::Pause {
                id: "timeout".into(),
            },
            None,
        )
        .await
    });
    let request = timeout(Duration::from_secs(3), requests.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(client.await.unwrap().unwrap_err().code(), "outcome-unknown");
    assert!(matches!(request.command, Command::Pause { ref id } if id == "timeout"));
    // Admission persists even though its result receiver timed out.
    assert!(request.reply.send(Ok(Response::Stopping)).is_err());
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn native_cleanup_drops_active_clients_and_allows_rebind() {
    let fixture = Fixture::new();
    let (management, _requests) = LocalManagement::channel();
    let server = tokio::spawn(
        Server::bind(&fixture.registry, management.clone())
            .unwrap()
            .run(),
    );
    let mut slow = connect_native(&fixture.registry).await;
    slow.write_u32(1000).await.unwrap();
    server.abort();
    let _ = server.await;
    // Client remains open; cancellation must nevertheless close server handles.
    let mut byte = [0];
    let result = timeout(Duration::from_secs(2), slow.read(&mut byte))
        .await
        .unwrap();
    assert!(matches!(result, Ok(0) | Err(_)));
    assert_eq!(
        execute(&fixture.registry, Command::Health, None)
            .await
            .unwrap_err()
            .code(),
        "runtime-not-running"
    );
    let rebound = Server::bind(&fixture.registry, management).unwrap();
    drop(slow);
    drop(rebound);
}

#[tokio::test]
async fn native_first_instance_rejects_squatting_or_a_second_server() {
    let fixture = Fixture::new();
    let (management, _requests) = LocalManagement::channel();
    let server = Server::bind(&fixture.registry, management.clone()).unwrap();
    assert!(Server::bind(&fixture.registry, management).is_err());
    drop(server);
}

#[tokio::test]
async fn native_shutdown_reply_is_written_before_server_finishes() {
    let fixture = Fixture::new();
    let (management, mut requests) = LocalManagement::channel();
    let server = tokio::spawn(Server::bind(&fixture.registry, management).unwrap().run());
    let registry = fixture.registry.clone();
    let client = tokio::spawn(async move { execute(&registry, Command::Shutdown, None).await });
    let request = timeout(Duration::from_secs(3), requests.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(request.command, Command::Shutdown));
    request.reply.send(Ok(Response::Stopping)).unwrap();
    timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        client.await.unwrap().unwrap().result.unwrap().r#type,
        "stopping"
    );
    assert_eq!(
        execute(&fixture.registry, Command::Health, None)
            .await
            .unwrap_err()
            .code(),
        "runtime-not-running"
    );
}

#[tokio::test]
async fn native_large_response_is_bounded_and_runtime_errors_are_redacted() {
    let fixture = Fixture::new();
    let (management, mut requests) = LocalManagement::channel();
    let server = tokio::spawn(Server::bind(&fixture.registry, management).unwrap().run());
    let registry = fixture.registry.clone();
    let client = tokio::spawn(async move { execute(&registry, Command::Health, None).await });
    let request = timeout(Duration::from_secs(3), requests.recv())
        .await
        .unwrap()
        .unwrap();
    let corpus: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/evidence-v2.json")).unwrap();
    let mut health: crate::evidence::RuntimeHealth =
        serde_json::from_value(corpus["runtime"].clone()).unwrap();
    health.runtime_id = "x".repeat(MAX_RESPONSE_BYTES);
    request.reply.send(Ok(Response::Health(health))).unwrap();
    assert_eq!(
        client.await.unwrap().unwrap_err().code(),
        "response-too-large"
    );

    let registry = fixture.registry.clone();
    let client = tokio::spawn(async move { execute(&registry, Command::Health, None).await });
    let request = timeout(Duration::from_secs(3), requests.recv())
        .await
        .unwrap()
        .unwrap();
    request
        .reply
        .send(Err(anyhow::anyhow!("a-secret-credential")))
        .unwrap();
    let error = client.await.unwrap().unwrap_err();
    assert_eq!(error.code(), "runtime-error");
    assert!(!error.to_string().contains("secret"));
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn client_response_deadline_reports_outcome_unknown() {
    let fixture = Fixture::new();
    let (management, mut requests) = LocalManagement::channel();
    let server = tokio::spawn(Server::bind(&fixture.registry, management).unwrap().run());
    let registry = fixture.registry.clone();
    let client = tokio::spawn(async move {
        let mut limits = testing_limits();
        limits.response = Duration::from_millis(100);
        execute_with_limits(&registry, Command::Health, None, limits).await
    });
    let request = timeout(Duration::from_secs(3), requests.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(client.await.unwrap().unwrap_err().code(), "outcome-unknown");
    assert!(request.reply.send(Ok(Response::Stopping)).is_ok());
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn write_deadline_bounds_an_unread_response() {
    let (mut server, _unread_client) = tokio::io::duplex(1);
    let error = timeout(
        Duration::from_millis(20),
        send_result(
            &mut server,
            Ok(Envelope::from_response(
                "shutdown",
                &Response::Stopping,
                None,
                0,
            )),
        ),
    )
    .await;
    assert!(error.is_err());
}
