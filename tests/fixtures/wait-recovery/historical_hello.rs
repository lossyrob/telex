pub fn daemon_capabilities() -> Vec<String> {
    let mut caps: Vec<String> = REQUIRED_CAPABILITIES
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    // Advertised-but-optional so it never breaks the required-capability handshake with an
    // older peer; provisioning code gates on it (and on `push_registered`) explicitly.
    caps.push(CAP_ON_DELIVER_EXEC.to_string());
    // Advertised-but-optional (issue #65): lets a client detect a daemon that understands the
    // deferred outcome + `DrainDeferred`, so version skew against an older daemon is diagnosable.
    caps.push(CAP_ON_DELIVER_DEFERRED.to_string());
    caps.push(CAP_APPLICATION_CLIENT_V1.to_string());
    caps.push(CAP_DELIVERY_QUARANTINE_V1.to_string());
    caps
}

pub fn daemon_required_capabilities() -> Vec<String> {
    REQUIRED_CAPABILITIES
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

pub fn client_hello(store_key: impl Into<String>) -> Hello {
    Hello {
        protocol_version: current_protocol_version(),
        client_version: DAEMON_VERSION.to_string(),
        store_key: store_key.into(),
        capabilities: daemon_capabilities(),
        required_capabilities: daemon_required_capabilities(),
        capability_scopes: Vec::new(),
    }
}
