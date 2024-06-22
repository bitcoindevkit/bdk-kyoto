//! bdk_kyoto

#![allow(unused)]
#![warn(missing_docs)]

use core::fmt;
use core::mem;
use tokio::sync::broadcast;

use bdk_wallet::bitcoin::{BlockHash, Transaction};

use bdk_wallet::chain::{
    collections::BTreeMap,
    keychain::KeychainTxOutIndex,
    local_chain::{self, CheckPoint},
    BlockId, ConfirmationTimeHeightAnchor, IndexedTxGraph,
};

use kyoto::node::{self, messages::NodeMessage};
use kyoto::IndexedBlock;
use kyoto::TxBroadcast;

/// Block height
type Height = u32;

/// Request.
#[derive(Debug)]
pub struct Request<'a, K> {
    cp: CheckPoint,
    index: &'a KeychainTxOutIndex<K>,
}

impl<'a, K> Request<'a, K>
where
    K: fmt::Debug + Clone + Ord,
{
    /// New.
    pub fn new(cp: CheckPoint, index: &'a KeychainTxOutIndex<K>) -> Self {
        Self { cp, index }
    }

    /// Into [`Client`].
    pub fn into_client(self, mut client: node::client::Client) -> Client<K> {
        let mut index = KeychainTxOutIndex::default();

        // clone the keychains given by the request
        for (keychain, descriptor) in self.index.keychains() {
            let _ = index.insert_descriptor(keychain.clone(), descriptor.clone());
        }

        let (sender, receiver) = client.split();

        Client {
            sender,
            receiver,
            chain_changeset: BTreeMap::new(),
            cp: self.cp,
            graph: IndexedTxGraph::new(index),
        }
    }
}

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
    /// Sync.
    pub async fn sync(&mut self) -> Option<Update<K>> {
        while let Ok(message) = self.receiver.recv().await {
            match message {
                NodeMessage::Block(IndexedBlock { height, block }) => {
                    let hash = block.header.block_hash();
                    self.chain_changeset.insert(height, Some(hash));

                    tracing::info!("Applying Block: {}", hash);
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
                NodeMessage::Synced(update) => {
                    if self.chain_changeset.is_empty()
                        && self.cp.height() == update.tip.height
                        && self.cp.hash() == update.tip.hash
                    {
                        // return early if we're already synced
                        tracing::info!("Done.");
                        return None;
                    }
                    self.chain_changeset.insert(update.tip.height, Some(update.tip.hash));

                    tracing::info!("Synced to tip {} {}", update.tip.height, update.tip.hash);
                    break;
                }
                NodeMessage::TxSent => {}
                NodeMessage::TxBroadcastFailure => {}
                NodeMessage::Dialog(s) => tracing::info!("{s}"),
                NodeMessage::Warning(s) => tracing::warn!("{s}"),
            }
        }

        let cp = self.update_chain()?;
        let indexed_tx_graph = mem::take(&mut self.graph);

        Some(Update {
            cp,
            indexed_tx_graph,
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
    pub async fn broadcast(&mut self, tx: &Transaction) -> Result<(), Error> {
        use kyoto::TxBroadcastPolicy::*;

        self.sender
            .broadcast_tx(TxBroadcast {
                tx: tx.clone(),
                broadcast_policy: AllPeers,
            })
            .await
            .map_err(Error::Client)
    }

    /// Shutdown.
    pub async fn shutdown(&mut self) -> Result<(), Error> {
        self.sender.shutdown().await.map_err(Error::Client)
    }
}

/// Update.
#[derive(Debug)]
pub struct Update<K> {
    /// `CheckPoint`
    pub cp: CheckPoint,
    /// `IndexedTxGraph`
    pub indexed_tx_graph: IndexedTxGraph<ConfirmationTimeHeightAnchor, KeychainTxOutIndex<K>>,
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
