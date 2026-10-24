//! Registered mTLS network adapter. Public strings never choose destinations.
use crate::HostPort;
use sandproof_api::PortError;
use sandproof_internal::InternalClient;
use sandproof_protocol::internal::NodeIdentity;
use std::{collections::BTreeSet, time::Duration};
use tokio::runtime::Handle;

#[derive(Clone)]
pub struct RegisteredHost {
    pub identity: NodeIdentity,
    pub client: InternalClient,
}
#[derive(Clone)]
pub struct RegisteredHosts {
    hosts: Vec<RegisteredHost>,
    pub(crate) runtime: Handle,
}
impl RegisteredHosts {
    /// Protected candidate preparation, never a public address/URL selector.
    /// # Errors
    /// Unknown results remain replayable for the SAME immutable cache request.
    pub fn cache_template(
        &self,
        request: &sandproof_internal::cache::CacheRequest,
        deadline: Duration,
    ) -> Result<sandproof_internal::cache::CacheAvailability, PortError> {
        let client = self.client(&request.node)?;
        self.runtime.block_on(async {
            tokio::time::timeout(deadline, client.cache_template(request))
                .await
                .map_err(|_| PortError::OutcomeUnknown)?
                .map_err(|_| PortError::OutcomeUnknown)
        })
    }
    /// # Errors
    /// Refuses unbounded, duplicate or cross-deployment registration.
    pub fn new(hosts: Vec<RegisteredHost>, runtime: Handle) -> Result<Self, PortError> {
        let Some(first) = hosts.first() else {
            return Err(PortError::Invalid);
        };
        if hosts.len() > 256
            || hosts
                .iter()
                .any(|host| host.identity != *host.client.node_identity())
            || hosts
                .iter()
                .any(|host| host.identity.deployment_id != first.identity.deployment_id)
            || hosts
                .iter()
                .map(|host| host.identity.node_id.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                != hosts.len()
        {
            return Err(PortError::Invalid);
        }
        Ok(Self { hosts, runtime })
    }
    pub(crate) fn client(&self, node: &NodeIdentity) -> Result<InternalClient, PortError> {
        self.hosts
            .iter()
            .find(|host| host.identity == *node)
            .map(|host| host.client.clone())
            .ok_or(PortError::Busy)
    }
    /// Registered read-only physical status, sharing the existing client budget.
    /// # Errors
    /// Unknown transport is not evidence of resource absence or VM termination.
    pub async fn status(
        &self,
        node: &NodeIdentity,
    ) -> Result<sandproof_internal::status::HostStatus, PortError> {
        self.client(node)?
            .status()
            .await
            .map_err(|_| PortError::Busy)
    }
}
impl HostPort for RegisteredHosts {
    fn confirm_lifecycle_renewal(
        &self,
        result: &sandproof_protocol::lifecycle::RenewalConfirmation,
        deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::ConfirmedRenewal, PortError> {
        let client = self.client(&result.request.request.binding.node)?;
        self.runtime.block_on(async {
            tokio::time::timeout(deadline, client.confirm_lifecycle_renewal(result))
                .await
                .map_err(|_| PortError::OutcomeUnknown)?
                .map_err(|_| PortError::OutcomeUnknown)
        })
    }
    fn confirm_lifecycle_terminal(
        &self,
        result: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
        deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::ConfirmedLifecycleResult, PortError> {
        let client = self.client(&result.request.request.binding.node)?;
        self.runtime.block_on(async {
            tokio::time::timeout(deadline, client.confirm_lifecycle_terminal(result))
                .await
                .map_err(|_| PortError::OutcomeUnknown)?
                .map_err(|_| PortError::OutcomeUnknown)
        })
    }
    fn close_lifecycle_window(
        &self,
        request: &sandproof_protocol::lifecycle::LifecycleWindowClose,
        deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::ClosedLifecycleWindow, PortError> {
        let client = self.client(&request.terminal.request.request.binding.node)?;
        self.runtime.block_on(async {
            tokio::time::timeout(deadline, client.close_lifecycle_window(request))
                .await
                .map_err(|_| PortError::OutcomeUnknown)?
                .map_err(|_| PortError::OutcomeUnknown)
        })
    }
    fn observe_lifecycle_unload(
        &self,
        intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::UnloadObservation, PortError> {
        let client = self.client(&intent.request.binding.node)?;
        self.runtime.block_on(async {
            tokio::time::timeout(deadline, client.observe_lifecycle_unload(intent)).await
                .map_err(|_| PortError::OutcomeUnknown)?.map_err(|_| PortError::OutcomeUnknown)
        })
    }
    fn observe_lifecycle_terminal(
        &self,
        intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::TerminalLifecycleObservation, PortError> {
        let client = self.client(&intent.request.binding.node)?;
        self.runtime.block_on(async {
            tokio::time::timeout(deadline, client.observe_lifecycle_terminal(intent))
                .await
                .map_err(|_| PortError::OutcomeUnknown)?
                .map_err(|_| PortError::OutcomeUnknown)
        })
    }
    fn observe_lifecycle_create(
        &self,
        intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::CreateObservation, PortError> {
        let client = self.client(&intent.request.binding.node)?;
        self.runtime.block_on(async {
            tokio::time::timeout(deadline, client.observe_lifecycle_create(intent))
                .await
                .map_err(|_| PortError::Busy)?
                .map_err(|_| PortError::Busy)
        })
    }
    fn record_lifecycle(
        &self,
        intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::RecordedLifecycleIntent, PortError> {
        let client = self.client(&intent.request.binding.node)?;
        self.runtime.block_on(async {
            tokio::time::timeout(deadline, client.record_lifecycle(intent))
                .await
                .map_err(|_| PortError::OutcomeUnknown)?
                .map_err(|_| PortError::OutcomeUnknown)
        })
    }
}
