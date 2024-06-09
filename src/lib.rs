//! bdk_kyoto

#![allow(unused)]
#![warn(missing_docs)]

use core::fmt;
use std::str::FromStr;

use bdk_chain::bitcoin::Block;
use bdk_chain::bitcoin::BlockHash;
use bdk_chain::bitcoin::Transaction;
use bdk_chain::collections::{BTreeMap, BTreeSet};
use bdk_chain::keychain::KeychainTxOutIndex;
use bdk_chain::local_chain::CheckPoint;
use bdk_chain::BlockId;
use bdk_chain::ConfirmationTimeHeightAnchor;
use bdk_chain::IndexedTxGraph;

use kyoto::node;
use kyoto::node::messages::NodeMessage;
use kyoto::Block as KyotoBlock;
use kyoto::BlockHash as KyotoBlockHash;
use kyoto::Transaction as KyotoTransaction;

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
    pub fn into_client(self, client: node::client::Client) -> Client<K> {
        let mut index = KeychainTxOutIndex::default();

        // clone the keychains given by the `Request`
        for (keychain, descriptor) in self.index.keychains() {
            let _ = index.insert_descriptor(keychain.clone(), descriptor.clone());
        }

        Client {
            inner: client,
            blocks: BTreeMap::new(),
            cp: self.cp,
            graph: IndexedTxGraph::new(index),
        }
    }
}

/// Client.
#[derive(Debug)]
pub struct Client<K> {
    inner: kyoto::node::client::Client,

    // blocks
    blocks: BTreeMap<u32, BlockHash>,
    // cp
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

        let (mut sender, mut receiver) = self.inner.split();

        while let Ok(message) = receiver.recv().await {
            match message {
                NodeMessage::Block(kyoto::IndexedBlock { height, block }) => {
                    let block = convert_block(&block);
                    let hash = block.header.block_hash();

                    tracing::info!("Applying Block..");
                    self.blocks.insert(height, hash);
                    let _ = self.graph.apply_block_relevant(&block, height);
                }
                NodeMessage::Transaction(_) => {}
                NodeMessage::BlocksDisconnected(_) => {
                    // what to do here?
                }
                NodeMessage::Synced(tip) => {
                    self.blocks.insert(tip.height, convert_hash(&tip.hash));
                    tracing::info!("Synced to tip {} {:?}", tip.height, tip.hash);
                    break;
                }
                NodeMessage::Dialog(s) => tracing::info!("{s}"),
                NodeMessage::Warning(s) => tracing::warn!("{s}"),
            }
        }

        tracing::info!("Shutting down");
        sender.shutdown().await.unwrap();

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
    pub fn shutdown(&mut self) {
        todo!()
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

/// Convert hash.
#[allow(dead_code)]
fn convert_hash(hash: &KyotoBlockHash) -> BlockHash {
    let s = hash.to_string();
    BlockHash::from_str(&s).unwrap()
}

/// Convert tx.
#[allow(dead_code)]
fn convert_tx(tx: &KyotoTransaction) -> Transaction {
    use bdk_chain::bitcoin::consensus::deserialize;
    use kyoto::consensus::serialize;
    let data = serialize(tx);
    deserialize(&data).expect("deserialize Transaction")
}

/// Convert block.
#[allow(dead_code)]
fn convert_block(block: &KyotoBlock) -> Block {
    use bdk_chain::bitcoin::consensus::deserialize;
    use kyoto::consensus::serialize;
    let data = serialize(block);
    deserialize(&data).expect("deserialize Transaction")
}
