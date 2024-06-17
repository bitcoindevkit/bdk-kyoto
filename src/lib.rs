//! bdk_kyoto

#![allow(unused)]
#![warn(missing_docs)]

use core::fmt;
use tokio::sync::broadcast;

use bdk_chain::bitcoin::BlockHash;
use bdk_chain::collections::{BTreeMap, BTreeSet};
use bdk_chain::keychain::KeychainTxOutIndex;
use bdk_chain::local_chain::CheckPoint;
use bdk_chain::BlockId;
use bdk_chain::ConfirmationTimeHeightAnchor;
use bdk_chain::IndexedTxGraph;

use kyoto::node;
use kyoto::node::messages::NodeMessage;
use kyoto::IndexedBlock;


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
        tracing::info!("Syncing..");

        while let Ok(message) = self.receiver.recv().await {
            match message {
                NodeMessage::Block(IndexedBlock { height, block }) => {
                    let hash = block.header.block_hash();

                    tracing::info!("Applying Block..");
                    self.blocks.insert(height, hash);
                    let _ = self.graph.apply_block_relevant(&block, height);
                }
                NodeMessage::Transaction(_) => {}
                NodeMessage::BlocksDisconnected(_) => {
                    // what to do here, just remove an entry from `self.blocks` ?
                }
                NodeMessage::Synced(tip) => {
                    self.blocks.insert(tip.height, tip.hash);
                    tracing::info!("Synced to tip {} {:?}", tip.height, tip.hash);
                    break;
                }
                NodeMessage::Dialog(s) => tracing::info!("{s}"),
                NodeMessage::Warning(s) => tracing::warn!("{s}"),
                //NodeMessage::TxBroadcastFailure
                _ => {}
            }
        }

        self.as_update()
    }

    /// As [`Update`].
    fn as_update(&mut self) -> Option<Update<K>> {
        let blocks: BTreeSet<BlockId> = self.blocks.iter().map(BlockId::from).collect();
        let min_update_height = blocks.iter().next()?.height;

        // find local block to base the new blocks onto
        let base: BlockId = {
            let mut iter = self.cp.iter();
            let mut cp = iter.next()?;
            while cp.block_id().height >= min_update_height {
                cp = iter.next()?;
            }
            cp.block_id()
        };

        let cp = CheckPoint::from_block_ids(core::iter::once(base).chain(blocks))
            .expect("blocks are well ordered");
        let indexed_tx_graph = core::mem::take(&mut self.graph);

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
