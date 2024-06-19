//! bdk_kyoto

#![allow(unused)]
#![warn(missing_docs)]

use core::fmt;
use core::mem;
use tokio::sync::broadcast;

use bdk_wallet::bitcoin::BlockHash;

use bdk_wallet::chain::{
    collections::{BTreeMap, BTreeSet},
    keychain::KeychainTxOutIndex,
    local_chain::CheckPoint,
    BlockId, ConfirmationTimeHeightAnchor, IndexedTxGraph,
};

use kyoto::chain::checkpoints::HeaderCheckpoint;
use kyoto::node;
use kyoto::node::messages::NodeMessage;
use kyoto::IndexedBlock;

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
            blocks: BTreeMap::new(),
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
    // blocks
    blocks: BTreeMap<u32, BlockHash>,
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
                    self.blocks.insert(height, hash);

                    tracing::info!("Applying Block..");
                    let _ = self.graph.apply_block_relevant(&block, height);
                }
                NodeMessage::Transaction(_) => {}
                NodeMessage::BlocksDisconnected(_) => {}
                NodeMessage::Synced(tip) => {
                    if self.blocks.is_empty()
                        && self.cp.height() == tip.height
                        && self.cp.hash() == tip.hash
                    {
                        // return early if we're already synced
                        tracing::info!("Done.");
                        return None;
                    }
                    self.blocks.insert(tip.height, tip.hash);

                    tracing::info!("Synced to tip {} {}", tip.height, tip.hash);
                    break;
                }
                NodeMessage::TxBroadcastFailure => {}
                NodeMessage::Dialog(s) => tracing::info!("{s}"),
                NodeMessage::Warning(s) => tracing::warn!("{s}"),
            }
        }

        let mut cp = self.cp.clone();
        for block in self.blocks.iter().map(BlockId::from) {
            cp = cp.insert(block);
        }
        let indexed_tx_graph = mem::take(&mut self.graph);

        Some(Update {
            cp,
            indexed_tx_graph,
        })
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
