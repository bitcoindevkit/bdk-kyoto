//! bdk_kyoto
#![warn(missing_docs)]

use core::fmt;
use std::collections::HashSet;
use std::sync::Arc;

use bdk_wallet::bitcoin::{ScriptBuf, Transaction};
use bdk_wallet::chain::{
    keychain::KeychainTxOutIndex,
    local_chain::{self, CheckPoint, LocalChain},
    spk_client::FullScanResult,
    ConfirmationTimeHeightAnchor, IndexedTxGraph,
};

use crate::logger::NodeMessageHandler;

pub mod builder;
pub mod logger;

pub use kyoto::{
    node::{
        self,
        client::{self, Receiver},
        messages::{NodeMessage, SyncUpdate},
        node::Node,
    },
    IndexedBlock, TrustedPeer, TxBroadcast, TxBroadcastPolicy,
};

/// Client.
#[derive(Debug)]
pub struct Client<K> {
    // channel sender
    sender: client::ClientSender,
    // channel receiver
    receiver: kyoto::node::client::Receiver<NodeMessage>,
    // changes to local chain
    chain: local_chain::LocalChain,
    // receive graph
    graph: IndexedTxGraph<ConfirmationTimeHeightAnchor, KeychainTxOutIndex<K>>,
    // messages
    message_handler: Arc<dyn NodeMessageHandler>,
}

impl<K> Client<K>
where
    K: fmt::Debug + Clone + Ord,
{
    /// Build a light client from a [`KeychainTxOutIndex`] and checkpoint
    pub fn from_index(
        cp: CheckPoint,
        index: &KeychainTxOutIndex<K>,
        client: client::Client,
    ) -> Self {
        let (sender, receiver) = client.split();
        Self {
            sender,
            receiver,
            chain: LocalChain::from_tip(cp).unwrap(),
            graph: IndexedTxGraph::new(index.clone()),
            message_handler: Arc::new(()),
        }
    }

    /// Add a logger to handle node messages
    pub fn set_logger(&mut self, logger: Arc<dyn NodeMessageHandler>) {
        self.message_handler = logger;
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
                    if chain_changeset.is_empty()
                        && self.chain.tip().height() == tip.height
                        && self.chain.tip().hash() == tip.hash
                    {
                        // return early if we're already synced
                        return None;
                    }
                    chain_changeset.insert(tip.height, Some(tip.hash));
                    break;
                }
                _ => (),
            }
        }
        self.chain.apply_changeset(&chain_changeset).unwrap();
        Some(FullScanResult {
            graph_update: self.graph.graph().clone(),
            chain_update: self.chain.tip(),
            last_active_indices: self.graph.index.last_used_indices(),
        })
    }

    // Send dialogs to an arbitrary logger
    fn handle_log(&self, message: &NodeMessage) {
        let logger = &self.message_handler;
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
            NodeMessage::BlocksDisconnected(_headers) => {}
            NodeMessage::TxSent(t) => {
                logger.handle_dialog(format!("Transaction sent: {t}"));
            }
            NodeMessage::TxBroadcastFailure(_r) => {},
            _ => (),
        }
    }

    /// Broadcast a [`Transaction`] with a [`TxBroadcastPolicy`] strategy.
    pub async fn broadcast(
        &self,
        tx: &Transaction,
        policy: TxBroadcastPolicy,
    ) -> Result<(), Error> {
        self.transaction_broadcaster().broadcast(tx, policy).await
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

/// Type that broadcasts transactions to the network.
#[derive(Debug)]
pub struct TransactionBroadcaster {
    sender: client::ClientSender,
}

impl TransactionBroadcaster {
    /// Create a new transaction broadcaster with the given client `sender`.
    pub fn new(sender: client::ClientSender) -> Self {
        Self { sender }
    }

    /// Broadcast a [`Transaction`] with a [`TxBroadcastPolicy`] strategy.
    pub async fn broadcast(
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
