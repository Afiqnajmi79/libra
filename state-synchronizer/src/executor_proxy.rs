// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::SynchronizerState;
use executor::Executor;
use failure::prelude::*;
use futures::{channel::oneshot, Future, FutureExt};
use grpcio::EnvBuilder;
use libra_config::config::NodeConfig;
use libra_types::{
    crypto_proxies::{LedgerInfoWithSignatures, ValidatorChangeEventWithProof, ValidatorVerifier},
    transaction::TransactionListWithProof,
};
use std::{pin::Pin, sync::Arc};
use storage_client::{StorageRead, StorageReadServiceClient};
use vm_runtime::MoveVM;

/// Proxies interactions with execution and storage for state synchronization
pub trait ExecutorProxyTrait: Sync + Send {
    /// Latest state (ledger info and synced version) in the local storage
    fn get_local_storage_state(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<SynchronizerState>> + Send>>;

    /// Execute and commit a batch of transactions
    fn execute_chunk(
        &self,
        txn_list_with_proof: TransactionListWithProof,
        ledger_info_with_sigs: LedgerInfoWithSignatures,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;

    /// Gets chunk of transactions given the known version, target version and the max limit.
    fn get_chunk(
        &self,
        known_version: u64,
        limit: u64,
        target_version: u64,
    ) -> Pin<Box<dyn Future<Output = Result<TransactionListWithProof>> + Send>>;

    fn validate_ledger_info(&self, target: &LedgerInfoWithSignatures) -> Result<()>;

    fn get_epoch_proof(&self, start_epoch: u64) -> Result<ValidatorChangeEventWithProof>;
}

pub(crate) struct ExecutorProxy {
    storage_read_client: Arc<StorageReadServiceClient>,
    executor: Arc<Executor<MoveVM>>,
    validator_verifier: ValidatorVerifier,
}

impl ExecutorProxy {
    pub(crate) fn new(executor: Arc<Executor<MoveVM>>, config: &NodeConfig) -> Self {
        let client_env = Arc::new(EnvBuilder::new().name_prefix("grpc-coord-").build());
        let storage_read_client = Arc::new(StorageReadServiceClient::new(
            client_env,
            &config.storage.address,
            config.storage.port,
        ));
        let validator_verifier = config.consensus.consensus_peers.get_validator_verifier();
        Self {
            storage_read_client,
            executor,
            validator_verifier,
        }
    }
}

fn convert_to_future<T: Send + 'static>(
    receiver: oneshot::Receiver<Result<T>>,
) -> Pin<Box<dyn Future<Output = Result<T>> + Send>> {
    async move {
        match receiver.await {
            Ok(Ok(t)) => Ok(t),
            Ok(Err(err)) => Err(format_err!("Failed to process request: {}", err)),
            Err(oneshot::Canceled) => {
                Err(format_err!("Executor Internal error: sender is dropped."))
            }
        }
    }
        .boxed()
}

impl ExecutorProxyTrait for ExecutorProxy {
    fn get_local_storage_state(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<SynchronizerState>> + Send>> {
        let client = Arc::clone(&self.storage_read_client);
        async move {
            let storage_info = client
                .get_startup_info_async()
                .await?
                .ok_or_else(|| format_err!("[state sync] Failed to access storage info"))?;
            let highest_synced_version = std::cmp::max(
                storage_info.ledger_info.ledger_info().version(),
                storage_info.synced_tree_state.map_or(0, |t| t.version),
            );
            Ok(SynchronizerState {
                highest_local_li: storage_info.ledger_info,
                highest_synced_version,
            })
        }
            .boxed()
    }

    fn execute_chunk(
        &self,
        txn_list_with_proof: TransactionListWithProof,
        ledger_info_with_sigs: LedgerInfoWithSignatures,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        convert_to_future(
            self.executor
                .execute_and_commit_chunk(txn_list_with_proof, ledger_info_with_sigs),
        )
    }

    fn get_chunk(
        &self,
        known_version: u64,
        limit: u64,
        target_version: u64,
    ) -> Pin<Box<dyn Future<Output = Result<TransactionListWithProof>> + Send>> {
        let client = Arc::clone(&self.storage_read_client);
        async move {
            client
                .get_transactions_async(known_version + 1, limit, target_version, false)
                .await
        }
            .boxed()
    }

    fn validate_ledger_info(&self, target: &LedgerInfoWithSignatures) -> Result<()> {
        target.verify(&self.validator_verifier)?;
        Ok(())
    }

    fn get_epoch_proof(&self, start_epoch: u64) -> Result<ValidatorChangeEventWithProof> {
        let ledger_info_per_epoch = self
            .storage_read_client
            .get_epoch_change_ledger_infos(start_epoch)?;
        Ok(ValidatorChangeEventWithProof::new(ledger_info_per_epoch))
    }
}
