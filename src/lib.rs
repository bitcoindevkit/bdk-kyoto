//! bdk_kyoto
#![allow(unused)]
#![warn(missing_docs)]

use bdk_wallet::bitcoin::{BlockHash, Transaction};
use bdk_wallet::chain::local_chain::LocalChain;
use bdk_wallet::chain::spk_client::FullScanResult;
use core::fmt;
use core::mem;
use kyoto::node::client::Receiver;
use logger::NodeMessageHandler;
use std::collections::HashSet;

use bdk_wallet::chain::{
    collections::BTreeMap,
    keychain::KeychainTxOutIndex,
    local_chain::{self, CheckPoint},
    BlockId, ConfirmationTimeHeightAnchor, IndexedTxGraph,
};

pub use kyoto::node::messages::SyncUpdate;
pub use kyoto::node::node::Node;
pub use kyoto::node::{self, messages::NodeMessage};
pub use kyoto::IndexedBlock;
pub use kyoto::ScriptBuf;
pub use kyoto::TrustedPeer;
pub use kyoto::TxBroadcast;
pub use kyoto::TxBroadcastPolicy;

pub mod builder;
pub mod logger;

/// Block height
type Height = u32;

/// Client.
#[derive(Debug)]
pub struct Client<K> {
    // channel sender
    sender: node::client::ClientSender,
    // channel receiver
    receiver: kyoto::node::client::Receiver<NodeMessage>,
    // changes to local chain
    chain: local_chain::LocalChain,
    // receive graph
    graph: IndexedTxGraph<ConfirmationTimeHeightAnchor, KeychainTxOutIndex<K>>,
    // messages
    message_handler: Option<Box<dyn NodeMessageHandler + Send + Sync + 'static>>,
}

impl<K> Client<K>
where
    K: fmt::Debug + Clone + Ord,
{
    /// Build a light client from an `KeychainTxOutIndex` and checkpoint
    pub fn from_index(
        cp: CheckPoint,
        index: &KeychainTxOutIndex<K>,
        client: node::client::Client,
    ) -> Self {
        let (sender, receiver) = client.split();
        Self {
            sender,
            receiver,
            chain: LocalChain::from_tip(cp).unwrap(),
            graph: IndexedTxGraph::new(index.clone()),
            message_handler: None,
        }
    }

    /// Add a logger to handle node messages
    pub fn set_logger(&mut self, logger: Box<dyn NodeMessageHandler + Send + Sync + 'static>) {
        self.message_handler = Some(logger)
    }

    /// Return the most recent update from the node once it has synced to the network's tip.
    /// This may take a significant portion of time during wallet recoveries or dormant wallets.
    pub async fn update(&mut self) -> Option<FullScanResult<K>> {
        let mut chain_changeset = local_chain::ChangeSet::new();
        while let Ok(message) = self.receiver.recv().await {
            self.handle_log(&message);
            match message {
                NodeMessage::Block(IndexedBlock { height, block }) => {
                    let hash = block.header.block_hash();
                    chain_changeset.insert(height, Some(hash));
                    let _ = self.graph.apply_block_relevant(&block, height);
                }
                NodeMessage::BlocksDisconnected(headers) => {
                    for header in headers {
                        let height = header.height;
                        chain_changeset.insert(height, None);
                    }
                }
                NodeMessage::Synced(SyncUpdate {
                    tip,
                    recent_history: _,
                }) => {
                    if chain_changeset.is_empty() {
                        // return early if we're already synced
                        return None;
                    }
                    break;
                }
                _ => (),
            }
        }
        self.chain.apply_changeset(&chain_changeset).unwrap();
        let indexed_tx_graph = mem::take(&mut self.graph);
        Some(FullScanResult {
            graph_update: indexed_tx_graph.graph().clone(),
            chain_update: self.chain.tip(),
            last_active_indices: indexed_tx_graph.index.last_used_indices(),
        })
    }

    // Send dialogs to an arbitrary logger
    fn handle_log(&self, message: &NodeMessage) {
        if let Some(logger) = &self.message_handler {
            match message {
                NodeMessage::Dialog(d) => logger.handle_dialog(d.clone()),
                NodeMessage::Warning(w) => logger.handle_warning(w.clone()),
                NodeMessage::StateChange(s) => logger.handle_state_change(*s),
                NodeMessage::Block(b) => {
                    let hash = b.block.header.block_hash();
                    logger.handle_dialog(format!("Applying Block: {hash}"));
                }
                NodeMessage::Synced(SyncUpdate {
                    tip,
                    recent_history: _,
                }) => {
                    logger.handle_dialog(format!("Synced to tip {} {}", tip.height, tip.hash));
                }
                NodeMessage::BlocksDisconnected(headers) => {
                    for header in headers {
                        let height = header.height;
                        logger.handle_warning(format!("Disconnecting block: {height}"));
                    }
                }
                NodeMessage::TxSent(t) => {
                    logger.handle_dialog(format!("Transaction sent: {t}"));
                }
                NodeMessage::TxBroadcastFailure(r) => {
                    logger.handle_warning(format!("Transaction rejected: {:?}", r.reason));
                }
                _ => (),
            }
        }
    }

    /// Broadcast a [`Transaction`] with a [`TxBroadcastPolicy`] strategy.
    pub async fn broadcast(
        &self,
        tx: &Transaction,
        policy: TxBroadcastPolicy,
    ) -> Result<(), Error> {
        self.sender
            .broadcast_tx(TxBroadcast {
                tx: tx.clone(),
                broadcast_policy: policy,
            })
            .await
            .map_err(Error::Sender)
    }

    /// Add more scripts to the node. Could this just check a SPK index?
    pub async fn add_scripts(&self, scripts: HashSet<ScriptBuf>) -> Result<(), Error> {
        self.sender
            .add_scripts(scripts)
            .await
            .map_err(Error::Sender)
    }

    /// Shutdown the node.
    pub async fn shutdown(&self) -> Result<(), Error> {
        self.sender.shutdown().await.map_err(Error::Sender)
    }

    /// Get a structure to broadcast transactions. Useful for broadcasting transactions and updating concurrently.
    pub fn transaction_broadcaster(&self) -> TransactionBroadcaster {
        TransactionBroadcaster::new(self.sender.clone())
    }

    /// Get a mutable reference to the underlying channel [`Receiver`].
    pub fn channel_receiver(&mut self) -> &mut Receiver<NodeMessage> {
        &mut self.receiver
    }
}

/// Broadcast transactions to the network.
#[derive(Debug)]
pub struct TransactionBroadcaster {
    sender: node::client::ClientSender,
}

impl TransactionBroadcaster {
    fn new(sender: node::client::ClientSender) -> Self {
        Self { sender }
    }

    /// Broadcast a [`Transaction`] with a [`TxBroadcastPolicy`] strategy.
    async fn broadcast(
        &mut self,
        tx: &Transaction,
        policy: TxBroadcastPolicy,
    ) -> Result<(), Error> {
        self.sender
            .broadcast_tx(TxBroadcast {
                tx: tx.clone(),
                broadcast_policy: policy,
            })
            .await
            .map_err(Error::Sender)
    }
}

/// Errors thrown by a client.
#[derive(Debug)]
pub enum Error {
    /// The channel to receive a message was closed. Likely the node has stopped running.
    Sender(node::error::ClientError),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sender(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}
