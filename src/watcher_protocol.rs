//! Provider-neutral v2 envelopes. Parse untrusted bytes through the `parse_*`
//! functions, not through `serde_json::Value`, which loses duplicate members.
use chrono::{DateTime, SecondsFormat, Utc};
use serde::de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 2;
pub const MIN_INTERVAL_SECONDS: u64 = 1;
pub const MAX_INTERVAL_SECONDS: u64 = 86_400;
pub const MIN_TIMEOUT_SECONDS: u64 = 1;
pub const MAX_TIMEOUT_SECONDS: u64 = 3_600;
pub const MAX_STDOUT_BYTES: usize = 256 * 1024;
pub const MAX_STDERR_BYTES: usize = 64 * 1024;
pub const MAX_STATE_BYTES: usize = 256 * 1024;
pub const MAX_PARAMETERS_BYTES: usize = MAX_STATE_BYTES;
pub const MAX_KIND_BYTES: usize = 256;
pub const MAX_SUBJECT_BYTES: usize = 512;
pub const MAX_BODY_BYTES: usize = 128 * 1024;
pub const MAX_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_NORMALIZED_METADATA_BYTES: usize = 80 * 1024;
pub const MAX_ADDRESS_BYTES: usize = 1024;
pub const MAX_BACKEND_PROFILE_BYTES: usize = 256;
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    Malformed(String),
    Canonicalization(String),
    OutputLimit(String),
}

impl ProtocolError {
    pub fn diagnostic_category(&self) -> &'static str {
        match self {
            Self::Malformed(_) => "malformed-output",
            Self::Canonicalization(_) => "canonicalization-failed",
            Self::OutputLimit(_) => "output-limit",
        }
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Malformed(message)
            | Self::Canonicalization(message)
            | Self::OutputLimit(message) => message,
        };
        write!(f, "{}: {message}", self.diagnostic_category())
    }
}

impl std::error::Error for ProtocolError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Registration {
    pub schema_version: u32,
    pub id: String,
    pub command: Vec<String>,
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    pub backend_profile: String,
    pub sender: String,
    pub target: String,
    #[serde(default = "default_attention")]
    pub attention: String,
    #[serde(default = "default_disposition")]
    pub requires_disposition: bool,
    // An empty default is resolved only by parse_registration's explicit cwd.
    #[serde(default)]
    pub working_directory: PathBuf,
    #[serde(default)]
    pub environment_allowlist: Vec<String>,
    #[serde(default = "empty_object", deserialize_with = "deserialize_value")]
    pub parameters: Value,
    #[serde(default, deserialize_with = "deserialize_value")]
    pub initial_state: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetectorRequest {
    pub schema_version: u32,
    pub attempt: AttemptRequest,
    pub watch: WatchRequest,
    #[serde(deserialize_with = "deserialize_value")]
    pub state: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptRequest {
    pub id: String,
    pub now: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WatchRequest {
    pub id: String,
    pub activated_at: String,
    #[serde(deserialize_with = "deserialize_last_success")]
    pub last_success_at: Option<String>,
    #[serde(deserialize_with = "deserialize_value")]
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetectorResult {
    pub schema_version: u32,
    pub outcome: Outcome,
    #[serde(
        default,
        deserialize_with = "deserialize_next_state",
        skip_serializing_if = "Option::is_none"
    )]
    pub next_state: Option<Value>,
    #[serde(
        default,
        deserialize_with = "deserialize_event",
        skip_serializing_if = "Option::is_none"
    )]
    pub event: Option<DetectorEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Idle,
    Event,
    Terminal,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectorEvent {
    pub kind: String,
    pub subject: String,
    pub body: String,
    #[serde(default = "empty_object", deserialize_with = "deserialize_object")]
    pub metadata: Value,
}

pub fn parse_registration(bytes: &[u8], cwd: &Path) -> Result<Registration, ProtocolError> {
    let value = parse_json(bytes)?;
    let supplied_directory = value.get("workingDirectory").is_some();
    let mut registration: Registration = from_value(value)?;
    if !supplied_directory {
        registration.working_directory = canonical_directory(cwd)?;
    }
    validate_registration(&registration)?;
    // Persist the physical path, including Windows' extended-length prefix.
    registration.working_directory = canonical_directory(&registration.working_directory)?;
    Ok(registration)
}

pub fn parse_request(bytes: &[u8]) -> Result<DetectorRequest, ProtocolError> {
    let request = from_value(parse_json(bytes)?)?;
    validate_request(&request)?;
    Ok(request)
}

/// Transport uses ordinary JSON: JCS can rewrite admitted floats as unsafe integer tokens.
pub fn encode_request(request: &DetectorRequest) -> Result<Vec<u8>, ProtocolError> {
    validate_request(request)?;
    let bytes = serde_json::to_vec(request).map_err(json_error)?;
    if parse_request(&bytes)? != *request {
        return malformed("request transport round-trip changed the request");
    }
    Ok(bytes)
}

pub fn parse_result(bytes: &[u8]) -> Result<DetectorResult, ProtocolError> {
    byte_cap("detector stdout", bytes.len(), MAX_STDOUT_BYTES)?;
    let result = from_value(parse_json(bytes)?)?;
    validate_result(&result)?;
    Ok(result)
}

pub fn validate_registration(registration: &Registration) -> Result<(), ProtocolError> {
    version(registration.schema_version)?;
    watch_id(&registration.id)?;
    if registration.command.is_empty()
        || registration.command[0].is_empty()
        || registration.command.iter().any(|arg| arg.contains('\0'))
    {
        return malformed("command must be direct argv with a nonempty executable and no NUL");
    }
    if !(MIN_INTERVAL_SECONDS..=MAX_INTERVAL_SECONDS).contains(&registration.interval_seconds) {
        return malformed("intervalSeconds must be between 1 and 86400");
    }
    if !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&registration.timeout_seconds) {
        return malformed("timeoutSeconds must be between 1 and 3600");
    }
    token(
        "backendProfile",
        &registration.backend_profile,
        MAX_BACKEND_PROFILE_BYTES,
    )?;
    token("sender", &registration.sender, MAX_ADDRESS_BYTES)?;
    token("target", &registration.target, MAX_ADDRESS_BYTES)?;
    if !matches!(
        registration.attention.as_str(),
        "interrupt" | "next-checkpoint" | "background" | "fyi"
    ) {
        return malformed("attention must be interrupt, next-checkpoint, background, or fyi");
    }
    let path = &registration.working_directory;
    let canonical = canonical_directory(path)?;
    if comparable_path(path) != comparable_path(&canonical) {
        return malformed("workingDirectory must be a canonical absolute directory");
    }
    let mut names = BTreeSet::new();
    for name in &registration.environment_allowlist {
        let mut chars = name.chars();
        if !matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
            || !chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
            || !names.insert(name)
        {
            return malformed("environmentAllowlist must contain unique valid variable names");
        }
    }
    json_cap("parameters", &registration.parameters, MAX_PARAMETERS_BYTES)?;
    json_cap("initialState", &registration.initial_state, MAX_STATE_BYTES)?;
    canonical_bytes(registration)?;
    Ok(())
}

pub fn validate_request(request: &DetectorRequest) -> Result<(), ProtocolError> {
    version(request.schema_version)?;
    non_control("attempt.id", &request.attempt.id)?;
    validate_timestamp(&request.attempt.now)?;
    watch_id(&request.watch.id)?;
    validate_timestamp(&request.watch.activated_at)?;
    if let Some(timestamp) = &request.watch.last_success_at {
        validate_timestamp(timestamp)?;
    }
    json_cap(
        "parameters",
        &request.watch.parameters,
        MAX_PARAMETERS_BYTES,
    )?;
    json_cap("state", &request.state, MAX_STATE_BYTES)?;
    canonical_bytes(request)?;
    Ok(())
}

pub fn validate_result(result: &DetectorResult) -> Result<(), ProtocolError> {
    version(result.schema_version)?;
    match result.outcome {
        Outcome::Idle if result.event.is_some() => return malformed("idle forbids event"),
        Outcome::Event if result.event.is_none() => return malformed("event requires event"),
        Outcome::Degraded if result.event.is_some() || result.next_state.is_some() => {
            return malformed("degraded forbids event and nextState");
        }
        _ => {}
    }
    if let Some(state) = &result.next_state {
        json_cap("nextState", state, MAX_STATE_BYTES)?;
    }
    if let Some(event) = &result.event {
        validate_event(event)?;
    }
    byte_cap(
        "detector stdout",
        serde_json::to_vec(result).map_err(json_error)?.len(),
        MAX_STDOUT_BYTES,
    )?;
    canonical_bytes(result)?;
    Ok(())
}

pub fn validate_event(event: &DetectorEvent) -> Result<(), ProtocolError> {
    non_control("event.kind", &event.kind)?;
    byte_cap("event.kind", event.kind.len(), MAX_KIND_BYTES)?;
    non_control("event.subject", &event.subject)?;
    byte_cap("event.subject", event.subject.len(), MAX_SUBJECT_BYTES)?;
    byte_cap("event.body", event.body.len(), MAX_BODY_BYTES)?;
    if !event.metadata.is_object() {
        return malformed("event.metadata must be a JSON object");
    }
    json_cap("event.metadata", &event.metadata, MAX_METADATA_BYTES)?;
    canonical_bytes(event)?;
    Ok(())
}

/// Hashable JSON uses binary64 numbers; integral JSON tokens must be safe
/// integers. Encode larger identifiers as strings, not lossy JSON integers.
pub fn canonical_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    // Preflight before the canonicalizer can round integers or collapse duplicate
    // keys emitted by a custom Serialize implementation.
    let raw = serde_json::to_vec(value)
        .map_err(|error| ProtocolError::Canonicalization(error.to_string()))?;
    parse_json(&raw)?;
    // Serialize the original value so NaN/Infinity are rejected, not converted
    // to null by serde_json's ordinary serializer.
    serde_json_canonicalizer::to_vec(&value)
        .map_err(|error| ProtocolError::Canonicalization(error.to_string()))
}

pub fn hash_value<T: Serialize + ?Sized>(value: &T) -> Result<String, ProtocolError> {
    let digest = Sha256::digest(canonical_bytes(value)?);
    use std::fmt::Write;
    let mut hash = String::with_capacity(71);
    hash.push_str("sha256:");
    for byte in digest {
        write!(hash, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(hash)
}

pub fn now_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn validate_timestamp(value: &str) -> Result<(), ProtocolError> {
    let bytes = value.as_bytes();
    let valid_length = bytes.len() == 20 || (22..=24).contains(&bytes.len());
    if !valid_length
        || bytes.last() != Some(&b'Z')
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || (bytes.len() > 20 && bytes.get(19) != Some(&b'.'))
    {
        return malformed("timestamp must be UTC with Z and zero to three fractional digits");
    }
    for (index, byte) in bytes.iter().enumerate().take(bytes.len() - 1) {
        if matches!(index, 4 | 7 | 10 | 13 | 16) || (index == 19 && bytes.len() > 20) {
            continue;
        }
        if !byte.is_ascii_digit() {
            return malformed("timestamp contains a non-digit");
        }
    }
    // Chrono accepts leap seconds, but the normative lexical schema does not.
    if &value[17..19] > "59" || DateTime::parse_from_rfc3339(value).is_err() {
        return malformed("timestamp is not a valid calendar date and time");
    }
    Ok(())
}

fn version(version: u32) -> Result<(), ProtocolError> {
    if version != SCHEMA_VERSION {
        return malformed("unsupported schemaVersion; expected 2");
    }
    Ok(())
}

fn watch_id(value: &str) -> Result<(), ProtocolError> {
    let mut bytes = value.bytes();
    if value.len() > 128
        || !matches!(bytes.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        || !bytes.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
    {
        return malformed("id must match [a-z0-9][a-z0-9._-]{0,127}");
    }
    Ok(())
}

fn non_control(field: &str, value: &str) -> Result<(), ProtocolError> {
    if value.is_empty() || value.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}') {
        return malformed(format!(
            "{field} must be nonempty with no ASCII control characters"
        ));
    }
    Ok(())
}

fn token(field: &str, value: &str, cap: usize) -> Result<(), ProtocolError> {
    non_control(field, value)?;
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return malformed(format!("{field} must not contain whitespace or controls"));
    }
    byte_cap(field, value.len(), cap)
}

fn canonical_directory(path: &Path) -> Result<PathBuf, ProtocolError> {
    let text = path
        .to_str()
        .ok_or_else(|| ProtocolError::Malformed("workingDirectory must be valid Unicode".into()))?;
    non_control("workingDirectory", text)?;
    if !path.is_absolute() {
        return malformed("workingDirectory must be absolute");
    }
    let canonical = path.canonicalize().map_err(|error| {
        ProtocolError::Malformed(format!("workingDirectory cannot be resolved: {error}"))
    })?;
    if !canonical.is_dir() {
        return malformed("workingDirectory must name an existing directory");
    }
    Ok(canonical)
}

fn comparable_path(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
    }
    #[cfg(not(windows))]
    text
}

fn byte_cap(field: &str, size: usize, cap: usize) -> Result<(), ProtocolError> {
    if size > cap {
        return Err(ProtocolError::OutputLimit(format!(
            "{field} exceeded {cap} UTF-8 bytes"
        )));
    }
    Ok(())
}

fn json_cap(field: &str, value: &Value, cap: usize) -> Result<(), ProtocolError> {
    let raw = serde_json::to_vec(value).map_err(json_error)?;
    byte_cap(field, raw.len(), cap)?;
    canonical_bytes(value)?;
    Ok(())
}

fn malformed<T>(message: impl Into<String>) -> Result<T, ProtocolError> {
    Err(ProtocolError::Malformed(message.into()))
}

fn from_value<T: DeserializeOwned>(value: Value) -> Result<T, ProtocolError> {
    serde_json::from_value(value).map_err(json_error)
}

#[doc(hidden)]
pub fn parse_json(bytes: &[u8]) -> Result<Value, ProtocolError> {
    std::str::from_utf8(bytes)
        .map_err(|error| ProtocolError::Canonicalization(error.to_string()))?;
    let value = serde_json::from_slice::<StrictValue>(bytes).map_err(json_error)?;
    validate_number_tokens(bytes)?;
    Ok(value.0)
}

fn json_error(error: serde_json::Error) -> ProtocolError {
    let message = error.to_string();
    if [
        "duplicate",
        "surrogate",
        "hex escape",
        "unicode",
        "Unicode",
        "noncharacter",
        "number out of range",
        "interoperable",
    ]
    .iter()
    .any(|needle| message.contains(needle))
    {
        ProtocolError::Canonicalization(message)
    } else {
        ProtocolError::Malformed(message)
    }
}

// The JSON parser has already checked syntax. Inspect numeric tokens before
// loss of lexical evidence (very large integers otherwise become f64).
fn validate_number_tokens(bytes: &[u8]) -> Result<(), ProtocolError> {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() && bytes[index] != b'"' {
                index += if bytes[index] == b'\\' { 2 } else { 1 };
            }
        } else if bytes[index] == b'-' || bytes[index].is_ascii_digit() {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_digit() || b".eE+-".contains(&bytes[index]))
            {
                index += 1;
            }
            let token = std::str::from_utf8(&bytes[start..index]).expect("validated UTF-8");
            let integer = !token.contains(['.', 'e', 'E']);
            let valid = if integer {
                token
                    .parse::<i64>()
                    .is_ok_and(|number| (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&number))
            } else {
                token.parse::<f64>().is_ok_and(|number| {
                    let mantissa = token.split(['e', 'E']).next().unwrap_or(token);
                    number.is_finite()
                        && (number != 0.0
                            || !mantissa.bytes().any(|byte| (b'1'..=b'9').contains(&byte)))
                })
            };
            if !valid {
                return Err(ProtocolError::Canonicalization(
                    "number is outside the interoperable binary64/safe-integer range".into(),
                ));
            }
            continue;
        }
        index += 1;
    }
    Ok(())
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("I-JSON without duplicate object members")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
                    return Err(E::custom("integer outside interoperable range"));
                }
                Ok(StrictValue(Value::Number(value.into())))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                if value > MAX_SAFE_INTEGER as u64 {
                    return Err(E::custom("integer outside interoperable range"));
                }
                Ok(StrictValue(Value::Number(value.into())))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(|number| StrictValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("number outside interoperable range"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                i_json_string(value)?;
                Ok(StrictValue(Value::String(value.to_owned())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                i_json_string(&value)?;
                Ok(StrictValue(Value::String(value)))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<StrictValue>()? {
                    values.push(value.0);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = object.next_key::<String>()? {
                    i_json_string(&key)?;
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("duplicate JSON object member"));
                    }
                    values.insert(key, object.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

fn i_json_string<E: de::Error>(value: &str) -> Result<(), E> {
    if value.chars().any(|character| {
        let code = u32::from(character);
        (0xfdd0..=0xfdef).contains(&code) || matches!(code & 0xffff, 0xfffe | 0xffff)
    }) {
        return Err(E::custom("I-JSON forbids Unicode noncharacters"));
    }
    Ok(())
}

fn deserialize_value<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Value, D::Error> {
    StrictValue::deserialize(deserializer).map(|value| value.0)
}

fn deserialize_next_state<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    deserialize_value(deserializer).map(Some)
}

fn deserialize_event<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<DetectorEvent>, D::Error> {
    DetectorEvent::deserialize(deserializer).map(Some)
}

fn deserialize_object<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Value, D::Error> {
    let value = deserialize_value(deserializer)?;
    if !value.is_object() {
        return Err(de::Error::custom("metadata must be an object"));
    }
    Ok(value)
}

fn deserialize_last_success<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

fn default_attention() -> String {
    "next-checkpoint".into()
}

fn default_disposition() -> bool {
    true
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}
