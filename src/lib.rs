//! bdk_kyoto
#![warn(missing_docs)]

use core::fmt;
use std::collections::{BTreeMap, HashSet};

use bdk_chain::{
    keychain_txout::KeychainTxOutIndex,
    local_chain::{self, CheckPoint, LocalChain},
    spk_client::FullScanResult,
    IndexedTxGraph,
};
use bdk_chain::{ConfirmationBlockTime, TxUpdate};
use kyoto::{IndexedBlock, TxBroadcast};

use crate::logger::NodeMessageHandler;

#[cfg(feature = "wallet")]
pub mod builder;
pub mod logger;

pub use bdk_chain::local_chain::MissingGenesisError;
pub use kyoto::{
    ClientError, DatabaseError, Node, NodeBuilder, NodeMessage, NodeState, Receiver, ScriptBuf,
    SyncUpdate, Transaction, TrustedPeer, TxBroadcastPolicy, Txid, Warning, MAINNET_HEADER_CP,
    SIGNET_HEADER_CP,
};

/// A compact block filter client.
#[derive(Debug)]
pub struct Client<K> {
    // channel sender
    sender: kyoto::ClientSender,
    // channel receiver
    receiver: kyoto::Receiver<NodeMessage>,
    // changes to local chain
    chain: local_chain::LocalChain,
    // receive graph
    graph: IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<K>>,
}

impl<K> Client<K>
where
    K: fmt::Debug + Clone + Ord,
{
    /// Build a light client from a [`KeychainTxOutIndex`] and checkpoint
    pub fn from_index(
        cp: CheckPoint,
        index: &KeychainTxOutIndex<K>,
        client: kyoto::Client,
    ) -> Result<Self, Error> {
        let (sender, receiver) = client.split();
        Ok(Self {
            sender,
            receiver,
            chain: LocalChain::from_tip(cp)?,
            graph: IndexedTxGraph::new(index.clone()),
        })
    }

    /// Return the most recent update from the node once it has synced to the network's tip.
    /// This may take a significant portion of time during wallet recoveries or dormant wallets.
    pub async fn update(&mut self, logger: &dyn NodeMessageHandler) -> Option<FullScanResult<K>> {
        let mut chain_changeset = BTreeMap::new();
        while let Ok(message) = self.receiver.recv().await {
            self.log(&message, logger);
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
                    recent_history,
                }) => {
                    if chain_changeset.is_empty()
                        && self.chain.tip().height() == tip.height
                        && self.chain.tip().hash() == tip.hash
                    {
                        // return early if we're already synced
                        return None;
                    }
                    recent_history.into_iter().for_each(|(height, header)| {
                        chain_changeset.insert(height, Some(header.block_hash()));
                    });
                    break;
                }
                _ => (),
            }
        }
        self.chain
            .apply_changeset(&local_chain::ChangeSet::from(chain_changeset))
            .expect("chain was initialized with genesis");
        let tx_update = TxUpdate::from(self.graph.graph().clone());
        let graph = core::mem::take(&mut self.graph);
        let last_active_indices = graph.index.last_used_indices();
        self.graph = IndexedTxGraph::new(graph.index);
        Some(FullScanResult {
            tx_update,
            last_active_indices,
            chain_update: Some(self.chain.tip()),
        })
    }

    // Send dialogs to an arbitrary logger
    fn log(&self, message: &NodeMessage, logger: &dyn NodeMessageHandler) {
        match message {
            NodeMessage::Dialog(d) => logger.dialog(d.clone()),
            NodeMessage::Warning(w) => logger.warning(w.clone()),
            NodeMessage::StateChange(s) => logger.state_changed(*s),
            NodeMessage::Block(b) => {
                let hash = b.block.header.block_hash();
                logger.dialog(format!("Applying Block: {hash}"));
            }
            NodeMessage::Synced(SyncUpdate {
                tip,
                recent_history: _,
            }) => {
                logger.synced(tip.height);
            }
            NodeMessage::BlocksDisconnected(headers) => {
                logger.blocks_disconnected(headers.iter().map(|dc| dc.height).collect());
            }
            NodeMessage::TxSent(t) => {
                // If this becomes a type in UniFFI then we can pass it to tx_sent
                logger.tx_sent(*t);
            }
            NodeMessage::TxBroadcastFailure(r) => logger.tx_failed(r.txid),
            NodeMessage::ConnectionsMet => logger.connections_met(),
            _ => (),
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
            .map_err(Error::from)
    }

    /// Add more scripts to the node. Could this just check a SPK index?
    pub async fn add_scripts(&self, scripts: HashSet<ScriptBuf>) -> Result<(), Error> {
        self.sender.add_scripts(scripts).await.map_err(Error::from)
    }

    /// Shutdown the node.
    pub async fn shutdown(&self) -> Result<(), Error> {
        self.sender.shutdown().await.map_err(Error::from)
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
    sender: kyoto::ClientSender,
}

impl TransactionBroadcaster {
    /// Create a new transaction broadcaster with the given client `sender`.
    fn new(sender: kyoto::ClientSender) -> Self {
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
            .map_err(Error::from)
    }
}

/// Errors thrown by a client.
#[derive(Debug)]
pub enum Error {
    /// The channel to receive a message was closed. Likely the node has stopped running.
    Sender(ClientError),
    /// The [`LocalChain`] provided has no genesis block.
    MissingGenesis(MissingGenesisError),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sender(e) => write!(
                f,
                "the receiving channel has been close. Is the node still running?: {e}"
            ),
            Self::MissingGenesis(e) => {
                write!(f, "the local chain provided has no genesis block: {e}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Sender(s) => Some(s),
            Error::MissingGenesis(m) => Some(m),
        }
    }
}

impl From<MissingGenesisError> for Error {
    fn from(value: MissingGenesisError) -> Self {
        Error::MissingGenesis(value)
    }
}

impl From<ClientError> for Error {
    fn from(value: ClientError) -> Self {
        Error::Sender(value)
    }
}
