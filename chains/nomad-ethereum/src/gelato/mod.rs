use ethers::signers::Signer;
use ethers::types::transaction::eip2718::TypedTransaction;
use ethers::types::{Address, H256};
use ethers::{prelude::Bytes, providers::Middleware};
use gelato_relay::{GelatoClient, RelayResponse, TaskState};
use nomad_core::{ChainCommunicationError, Signers, TxOutcome};
use std::{str::FromStr, sync::Arc};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::info;
use utils::CHAIN_ID_TO_FORWARDER;

/// EIP-712 forward request structure
mod types;
pub use types::*;

pub mod utils;

pub(crate) const GAS_TANK_PAYMENT: usize = 1;

pub(crate) const ACCEPTABLE_STATES: [TaskState; 4] = [
    TaskState::CheckPending,
    TaskState::ExecPending,
    TaskState::ExecSuccess,
    TaskState::WaitingForConfirmation,
];

/// Gelato client for submitting txs to single chain
#[derive(Debug, Clone)]
pub struct SingleChainGelatoClient<M> {
    /// Gelato client
    pub gelato: Arc<GelatoClient>,
    /// Ethers client (for estimating gas)
    pub eth_client: Arc<M>,
    /// Sponsor signer
    pub sponsor: Signers,
    /// Gelato relay forwarder address
    pub forwarder: Address,
    /// Chain id
    pub chain_id: usize,
    /// Fee token
    pub fee_token: String,
    /// Transactions are of high priority
    pub is_high_priority: bool,
}

impl<M> SingleChainGelatoClient<M>
where
    M: Middleware + 'static,
{
    /// Get reference to base client
    pub fn gelato(&self) -> Arc<GelatoClient> {
        self.gelato.clone()
    }

    /// Instantiate single chain client with default Gelato url
    pub fn with_default_url(
        eth_client: Arc<M>,
        sponsor: Signers,
        chain_id: usize,
        fee_token: String,
        is_high_priority: bool,
    ) -> Self {
        Self {
            gelato: GelatoClient::default().into(),
            eth_client,
            sponsor,
            forwarder: *CHAIN_ID_TO_FORWARDER
                .get(&chain_id)
                .expect("!forwarder proxy"),
            chain_id,
            fee_token,
            is_high_priority,
        }
    }

    /// Submit a transaction to Gelato and poll until completion or failure.
    pub async fn submit_blocking(
        &self,
        domain: u32,
        contract_address: Address,
        tx: &TypedTransaction,
    ) -> Result<TxOutcome, ChainCommunicationError> {
        let RelayResponse { task_id } = self.dispatch_tx(domain, contract_address, tx).await?;

        info!(task_id = ?&task_id, "Submitted tx to Gelato relay.");

        info!(task_id = ?&task_id, "Polling Gelato task...");
        self.poll_task_id(task_id)
            .await
            .map_err(|e| ChainCommunicationError::TxSubmissionError(e.into()))?
    }

    /// Dispatch tx to Gelato and return task id.
    pub async fn dispatch_tx(
        &self,
        domain: u32,
        contract_address: Address,
        tx: &TypedTransaction,
    ) -> Result<RelayResponse, ChainCommunicationError> {
        // If gas limit not hardcoded in tx, eth_estimateGas
        let gas_limit = tx
            .gas()
            .unwrap_or(
                &self
                    .eth_client
                    .estimate_gas(tx)
                    .await
                    .map_err(|e| ChainCommunicationError::MiddlewareError(e.into()))?,
            )
            .as_usize();
        let data = tx.data().expect("!tx data");

        info!(
            domain = domain,
            contract_address = ?contract_address,
            "Dispatching tx to Gelato relay."
        );

        self.send_forward_request(contract_address, data, gas_limit)
            .await
            .map_err(|e| ChainCommunicationError::TxSubmissionError(e.into()))
    }

    /// Poll task id and return tx hash of transaction if successful, error if
    /// otherwise.
    pub fn poll_task_id(
        &self,
        task_id: String,
    ) -> JoinHandle<Result<TxOutcome, ChainCommunicationError>> {
        let gelato = self.gelato();

        tokio::spawn(async move {
            loop {
                let status = gelato
                    .get_task_status(&task_id)
                    .await
                    .map_err(|e| ChainCommunicationError::TxSubmissionError(e.into()))?
                    .expect("!task status");

                if !ACCEPTABLE_STATES.contains(&status.task_state) {
                    return Err(ChainCommunicationError::TxSubmissionError(
                        format!("Gelato task failed: {:?}", status).into(),
                    ));
                }

                if let Some(execution) = &status.execution {
                    info!(
                        chain = ?status.chain,
                        task_id = ?status.task_id,
                        execution = ?execution,
                        "Gelato relay executed tx."
                    );

                    let tx_hash = &execution.transaction_hash;
                    let txid = H256::from_str(tx_hash)
                        .unwrap_or_else(|_| panic!("Malformed tx hash from Gelato"));

                    return Ok(TxOutcome { txid });
                }

                if status.task_state == TaskState::CheckPending {
                    info!(status = ?status, "Polling pending Gelato task...");
                }

                sleep(Duration::from_secs(3)).await;
            }
        })
    }

    /// Format and sign forward request, then dispatch to Gelato relay service.
    pub async fn send_forward_request(
        &self,
        target: Address,
        data: &Bytes,
        gas_limit: usize,
    ) -> Result<RelayResponse, ChainCommunicationError> {
        let max_fee = self
            .gelato()
            .get_estimated_fee(self.chain_id, &self.fee_token, gas_limit + 100_000, false)
            .await
            .map_err(|e| ChainCommunicationError::CustomError(e.into()))?; // add 100k gas padding for Gelato contract ops

        let target = format!("{:#x}", target);
        let sponsor = format!("{:#x}", self.sponsor.address());
        let data = data.to_string().strip_prefix("0x").unwrap().to_owned();

        let unfilled_request = UnfilledForwardRequest {
            chain_id: self.chain_id,
            target,
            data,
            fee_token: self.fee_token.to_owned(),
            payment_type: GAS_TANK_PAYMENT, // gas tank
            max_fee,
            gas: gas_limit,
            sponsor,
            sponsor_chain_id: self.chain_id,
            nonce: 0,                     // default, not needed
            enforce_sponsor_nonce: false, // replay safety builtin to contracts
            enforce_sponsor_nonce_ordering: false,
        };

        info!(request = ?unfilled_request, "Signing gelato forward request.");

        let sponsor_signature = self
            .sponsor
            .sign_typed_data(&unfilled_request)
            .await
            .unwrap();

        let filled_request = unfilled_request.into_filled(sponsor_signature.to_vec());

        info!(request = ?filled_request, "Signed gelato forward request.");

        self.gelato()
            .send_forward_request(&filled_request)
            .await
            .map_err(|e| ChainCommunicationError::TxSubmissionError(e.into()))
    }
}
