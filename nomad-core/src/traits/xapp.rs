use crate::{
    traits::{ChainCommunicationError, CheckedTxOutcome},
    NomadIdentifier, SignedFailureNotification,
};
use async_trait::async_trait;

/// Interface for on-chain XAppConnectionManager
#[async_trait]
pub trait ConnectionManager: Send + Sync + std::fmt::Debug {
    /// Return the contract's local domain ID
    fn local_domain(&self) -> u32;

    /// Returns true if provided address is enrolled replica
    async fn is_replica(&self, address: NomadIdentifier) -> Result<bool, ChainCommunicationError>;

    /// Returns permission for address at given domain
    async fn watcher_permission(
        &self,
        address: NomadIdentifier,
        domain: u32,
    ) -> Result<bool, ChainCommunicationError>;

    /// onlyOwner function. Enrolls replica at given domain chain.
    async fn owner_enroll_replica(
        &self,
        replica: NomadIdentifier,
        domain: u32,
    ) -> Result<CheckedTxOutcome, ChainCommunicationError>;

    /// onlyOwner function. Unenrolls replica.
    async fn owner_unenroll_replica(
        &self,
        replica: NomadIdentifier,
    ) -> Result<CheckedTxOutcome, ChainCommunicationError>;

    /// onlyOwner function. Sets contract's home to provided home.
    async fn set_home(
        &self,
        home: NomadIdentifier,
    ) -> Result<CheckedTxOutcome, ChainCommunicationError>;

    /// onlyOwner function. Sets permission for watcher at given domain.
    async fn set_watcher_permission(
        &self,
        watcher: NomadIdentifier,
        domain: u32,
        access: bool,
    ) -> Result<CheckedTxOutcome, ChainCommunicationError>;

    /// Unenroll the replica at the given domain provided an updater address
    /// and `SignedFailureNotification` from a watcher
    async fn unenroll_replica(
        &self,
        signed_failure: &SignedFailureNotification,
    ) -> Result<CheckedTxOutcome, ChainCommunicationError>;
}
