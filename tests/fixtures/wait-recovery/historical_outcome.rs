#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaiterOutcome {
    Message,
    DeliveryQuarantined,
    IdleTimeout,
    PresenceEnded,
    AbnormalExit,
}
