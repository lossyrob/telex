//! Watcher policy over the supported Application Client; no daemon or backend access.

use crate::evidence::{SenderHealth, Watch};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
use telex::application_client::{
    AddressSpec, ApplicationCapability, ApplicationClient, ApplicationClientConfig,
    ApplicationClientError, ApplicationDaemonBootstrap, ApplicationHealth,
    ApplicationResponsibility, CompensationAction, MultiAddressOutcome, RecoveryPolicy,
};

pub(crate) const CLIENT_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) struct Clients {
    responsibility: String,
    bootstrap: ApplicationDaemonBootstrap,
    profiles: BTreeMap<String, Arc<ApplicationClient>>,
    known: BTreeMap<String, BTreeSet<String>>,
    pub health: BTreeMap<String, SenderHealth>,
    pub reconciliation_needed: bool,
}

impl Clients {
    pub fn new(responsibility: String, bootstrap: ApplicationDaemonBootstrap) -> Self {
        Self {
            responsibility,
            bootstrap,
            profiles: BTreeMap::new(),
            known: BTreeMap::new(),
            health: BTreeMap::new(),
            reconciliation_needed: true,
        }
    }

    async fn connect(
        &mut self,
        profile: &str,
    ) -> Result<Arc<ApplicationClient>, ApplicationClientError> {
        if let Some(client) = self.profiles.get(profile) {
            return Ok(client.clone());
        }
        let client = bounded(ApplicationClient::connect_with_daemon(
            ApplicationClientConfig {
                responsibility: ApplicationResponsibility(self.responsibility.clone()),
                backend: Some(profile.to_owned()),
                db_override: None,
            },
            self.bootstrap.clone(),
        ))
        .await?;
        // Profile aliases for one logical store share one application session.
        let client = self
            .profiles
            .values()
            .find(|existing| existing.logical_store_id() == client.logical_store_id())
            .cloned()
            .unwrap_or_else(|| Arc::new(client));
        self.profiles.insert(profile.to_owned(), client.clone());
        Ok(client)
    }

    pub fn get(&self, profile: &str) -> Option<Arc<ApplicationClient>> {
        self.profiles.get(profile).cloned()
    }

    pub fn ready(&self, profile: &str, sender: &str, store: Option<&str>) -> bool {
        self.health.get(profile).is_some_and(|health| {
            health.ready
                && store.is_some_and(|store| health.logical_store_id.as_deref() == Some(store))
                && health
                    .membership
                    .iter()
                    .any(|m| m.address == sender && m.sender_ready)
        })
    }

    pub fn invalidate(&mut self, profile: &str, error: ApplicationClientError) {
        self.reconciliation_needed = true;
        if let Some(health) = self.health.get_mut(profile) {
            health.ready = false;
            health.error = Some(error);
        }
    }

    pub async fn reconcile(&mut self, watches: &[Watch]) {
        self.reconciliation_needed = false;
        let mut desired: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for watch in watches {
            if let Some(registration) = &watch.registration {
                if watch.lifecycle != crate::evidence::Lifecycle::Removed
                    || watch.pending_operation_id.is_some()
                {
                    desired
                        .entry(registration.backend_profile.clone())
                        .or_default()
                        .insert(registration.sender.clone());
                }
            }
        }
        // Retain former profiles long enough to detach removed responsibilities.
        for profile in self.profiles.keys() {
            desired.entry(profile.clone()).or_default();
        }
        for profile in desired.keys() {
            if let Err(error) = self.connect(profile).await {
                self.health.insert(
                    profile.clone(),
                    SenderHealth {
                        backend_profile: profile.clone(),
                        logical_store_id: None,
                        ready: false,
                        membership: Vec::new(),
                        lifecycle: None,
                        compensation: Vec::new(),
                        error: Some(error),
                    },
                );
            }
        }
        let mut processed = BTreeSet::new();
        for profile in desired.keys() {
            let Some(client) = self.get(profile) else {
                continue;
            };
            let store = client.logical_store_id().0.clone();
            if !processed.insert(store.clone()) {
                continue;
            }
            let aliases: Vec<_> = desired
                .keys()
                .filter(|name| {
                    self.profiles
                        .get(*name)
                        .is_some_and(|c| c.logical_store_id() == client.logical_store_id())
                })
                .cloned()
                .collect();
            let addresses: BTreeSet<String> = aliases
                .iter()
                .flat_map(|name| desired[name].iter().cloned())
                .collect();
            let known = self.known.entry(store.clone()).or_default();
            let removed: Vec<_> = known.difference(&addresses).cloned().collect();
            let mut detach_failed = None;
            if !removed.is_empty() {
                let mut operation = client.begin_detach_many(&removed);
                let outcome = match tokio::time::timeout(CLIENT_TIMEOUT, operation.run()).await {
                    Ok(outcome) => outcome,
                    Err(_) => operation.cancelled_outcome(),
                };
                if outcome.ready {
                    for address in &removed {
                        known.remove(address);
                    }
                } else {
                    detach_failed = Some(outcome);
                }
            }
            let specs: Vec<_> = addresses
                .iter()
                .map(|address| AddressSpec {
                    address: address.clone(),
                    capability: ApplicationCapability::SendOnly,
                    description: Some("Telex Watcher send-only responsibility".to_owned()),
                    scope: None,
                    tags: None,
                })
                .collect();
            let (new_specs, old_specs): (Vec<_>, Vec<_>) = specs
                .into_iter()
                .partition(|spec| !known.contains(&spec.address));
            let mut operation = client.begin_attach(&new_specs);
            let mut outcome = match tokio::time::timeout(CLIENT_TIMEOUT, operation.run()).await {
                Ok(outcome) => outcome,
                Err(_) => operation.cancelled_outcome(),
            };
            if let Some(cancelled) = &mut outcome.cancellation {
                cancelled
                    .not_attempted
                    .extend(old_specs.iter().map(|spec| spec.address.clone()));
            } else if !old_specs.is_empty() {
                let mut operation = client
                    .begin_reconcile_many(&old_specs, RecoveryPolicy::BoundedRepair { retries: 1 });
                let reconciled = match tokio::time::timeout(CLIENT_TIMEOUT, operation.run()).await {
                    Ok(outcome) => outcome,
                    Err(_) => operation.cancelled_outcome(),
                };
                outcome.ready &= reconciled.ready;
                outcome.results.extend(reconciled.results);
                outcome.compensation.extend(reconciled.compensation);
                outcome.validation_error = outcome.validation_error.or(reconciled.validation_error);
                // Only one lifecycle operation can be uncertain: do not begin
                // another one after cancellation of the initial attach.
                outcome.cancellation = outcome.cancellation.or(reconciled.cancellation);
            }
            known.extend(addresses.iter().cloned());
            // Partial startup is compensated with the exact public handles.
            // The outcome itself is retained even if compensation succeeds.
            let mut compensation_outcomes = Vec::new();
            if !outcome.ready {
                for compensation in &outcome.compensation {
                    match &compensation.action {
                        CompensationAction::Detach => {
                            let addresses = vec![compensation.address.clone()];
                            let mut repair = client.begin_detach_many(&addresses);
                            let result =
                                match tokio::time::timeout(CLIENT_TIMEOUT, repair.run()).await {
                                    Ok(result) => result,
                                    Err(_) => repair.cancelled_outcome(),
                                };
                            if result.ready {
                                known.remove(&compensation.address);
                            }
                            compensation_outcomes.push(result);
                        }
                        CompensationAction::Reattach(spec) => {
                            let mut repair = client.begin_attach(std::slice::from_ref(spec));
                            let evidence =
                                match tokio::time::timeout(CLIENT_TIMEOUT, repair.run()).await {
                                    Ok(result) => result,
                                    Err(_) => repair.cancelled_outcome(),
                                };
                            compensation_outcomes.push(evidence);
                        }
                    }
                }
            }
            let (membership, error) = match bounded(client.health()).await {
                Ok(membership) => (membership, None),
                Err(error) => (Vec::new(), Some(error)),
            };
            let ready = outcome.ready
                && detach_failed.is_none()
                && error.is_none()
                && addresses.iter().all(|address| {
                    membership.iter().any(|member| {
                        member.address == *address && membership_ready(&client, member)
                    })
                });
            for alias in aliases {
                self.health.insert(
                    alias.clone(),
                    SenderHealth {
                        backend_profile: alias,
                        logical_store_id: Some(store.clone()),
                        ready,
                        membership: membership.clone(),
                        lifecycle: Some(detach_failed.clone().unwrap_or_else(|| outcome.clone())),
                        compensation: compensation_outcomes.clone(),
                        error: error.clone(),
                    },
                );
            }
        }
    }

    pub async fn shutdown(&mut self) -> Vec<MultiAddressOutcome> {
        let mut outcomes = Vec::new();
        let mut processed = BTreeSet::new();
        for client in self.profiles.values() {
            let store = client.logical_store_id().0.clone();
            if !processed.insert(store.clone()) {
                continue;
            }
            let addresses: Vec<_> = self
                .known
                .get(&store)
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            let mut operation = client.begin_detach_many(&addresses);
            let outcome = match tokio::time::timeout(CLIENT_TIMEOUT, operation.run()).await {
                Ok(result) => result,
                Err(_) => operation.cancelled_outcome(),
            };
            outcomes.push(outcome);
        }
        outcomes
    }
}

pub(crate) fn membership_ready(client: &ApplicationClient, member: &ApplicationHealth) -> bool {
    member.sender_ready
        && member.capability == ApplicationCapability::SendOnly
        && member.logical_store_id == *client.logical_store_id()
        && member.runtime_id == *client.runtime_id()
        && member.responsibility == *client.responsibility()
        && member.registered
        && !member.recovering
        && member.inbound_actionable == 0
        && !member.receive_ready
        && member.liveness.iter().any(|proof| {
            proof.pid == std::process::id() && proof.start_time.is_some() && proof.alive
        })
}

pub(crate) async fn bounded<T>(
    future: impl std::future::Future<Output = Result<T, ApplicationClientError>>,
) -> Result<T, ApplicationClientError> {
    tokio::time::timeout(CLIENT_TIMEOUT, future)
        .await
        .unwrap_or_else(|_| {
            Err(ApplicationClientError::TransportUncertain(
                "Watcher client observation timed out".into(),
            ))
        })
}
