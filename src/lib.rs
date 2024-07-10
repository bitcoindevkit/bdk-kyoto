//! bdk_kyoto
#![allow(unused)]
#![warn(missing_docs)]

use bdk_wallet::chain::spk_client::FullScanResult;
use core::fmt;
use core::mem;
use kyoto::node::client::Receiver;
use std::collections::HashSet;
pub use tokio::sync::broadcast;

use bdk_wallet::bitcoin::{BlockHash, Transaction};

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

/// Block height
type Height = u32;

/// Client.
#[derive(Debug)]
pub struct Client<K> {
    // channel sender
    sender: node::client::ClientSender,
    // channel receiver
    receiver: broadcast::Receiver<NodeMessage>,
    // changes to local chain
    chain_changeset: local_chain::ChangeSet,
    // local cp
    cp: CheckPoint,
    // receive graph
    graph: IndexedTxGraph<ConfirmationTimeHeightAnchor, KeychainTxOutIndex<K>>,
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
            chain_changeset: BTreeMap::new(),
            cp,
            graph: IndexedTxGraph::new(index.clone()),
        }
    }

    /// Sync.
    pub async fn update(&mut self) -> Option<FullScanResult<K>> {
        while let Ok(message) = self.receiver.recv().await {
            match message {
                NodeMessage::Block(IndexedBlock { height, block }) => {
                    let hash = block.header.block_hash();
                    self.chain_changeset.insert(height, Some(hash));
                    tracing::info!("Applying Block: {hash}");
                    let _ = self.graph.apply_block_relevant(&block, height);
                }
                NodeMessage::Transaction(_) => {}
                NodeMessage::BlocksDisconnected(headers) => {
                    for header in headers {
                        let height = header.height;
                        tracing::info!("Disconnecting block: {height}");
                        self.chain_changeset.insert(height, None);
                    }
                }
                NodeMessage::Synced(SyncUpdate {
                    tip,
                    recent_history: _,
                }) => {
                    if self.chain_changeset.is_empty()
                        && self.cp.height() == tip.height
                        && self.cp.hash() == tip.hash
                    {
                        // return early if we're already synced
                        tracing::info!("No updates.");
                        return None;
                    }
                    self.chain_changeset.insert(tip.height, Some(tip.hash));

                    tracing::info!("Synced to tip {} {}", tip.height, tip.hash);
                    break;
                }
                NodeMessage::StateChange(s) => {
                    tracing::info!("The node changed state to: {s:?}");
                }
                NodeMessage::TxSent(_) => {}
                NodeMessage::TxBroadcastFailure(_) => {}
                NodeMessage::Dialog(s) => {
                    tracing::info!("{s}")
                }
                NodeMessage::Warning(s) => {
                    tracing::warn!("{s}")
                }
            }
        }

        let cp = self.update_chain()?;
        let indexed_tx_graph = mem::take(&mut self.graph);
        Some(FullScanResult {
            graph_update: indexed_tx_graph.graph().clone(),
            chain_update: cp,
            last_active_indices: indexed_tx_graph.index.last_used_indices(),
        })
    }

    /// Update chain.
    fn update_chain(&self) -> Option<CheckPoint> {
        // Note: this is taken from `CheckPoint::apply_changeset`
        // which is not currently a pub fn
        let start_height = self.chain_changeset.keys().copied().next()?;
        let mut blocks = BTreeMap::<Height, BlockHash>::new();

        // find local block to base new blocks onto, keeping all
        // original heights above `start_height`
        let base: BlockId = {
            let mut it = self.cp.iter();
            let mut cp = it.next()?;
            while cp.height() >= start_height {
                blocks.insert(cp.height(), cp.hash());
                cp = it.next().expect("fallback to genesis");
            }
            cp.block_id()
        };

        // apply the changes
        for (&height, &hash) in self.chain_changeset.iter() {
            match hash {
                Some(hash) => {
                    blocks.insert(height, hash);
                }
                None => {
                    blocks.remove(&height);
                }
            };
        }

        Some(
            CheckPoint::from_block_ids(
                core::iter::once(base).chain(blocks.into_iter().map(BlockId::from)),
            )
            .expect("blocks are well ordered"),
        )
    }

    /// Broadcast a [`Transaction`].
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
            .map_err(Error::Client)
    }

    /// Add more scripts to the node. Could this just check a SPK index?
    pub async fn add_scripts(&self, scripts: HashSet<ScriptBuf>) -> Result<(), Error> {
        self.sender
            .add_scripts(scripts)
            .await
            .map_err(Error::Client)
    }

    /// Shutdown.
    pub async fn shutdown(&self) -> Result<(), Error> {
        self.sender.shutdown().await.map_err(Error::Client)
    }

    /// Get a struct to broadcast transactions
    pub fn transaction_broadcaster(&self) -> TransactionBroadcaster {
        TransactionBroadcaster::new(self.sender.clone())
    }

    /// Get the underlying channel receiver
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
            .map_err(Error::Client)
    }
}

/// Crate error.
#[derive(Debug)]
pub enum Error {
    /// Client
    Client(node::error::ClientError),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}
