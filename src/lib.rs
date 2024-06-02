//! bdk_kyoto

#![warn(missing_docs)]

use core::fmt;
use std::str::FromStr;

use bdk_chain::bitcoin::BlockHash;
use bdk_chain::bitcoin::Transaction;
use bdk_chain::collections::{BTreeMap, HashSet};
use bdk_chain::keychain::KeychainTxOutIndex;
use bdk_chain::local_chain::CheckPoint;
use bdk_chain::BlockId;
use bdk_chain::ConfirmationTimeHeightAnchor;
use bdk_chain::IndexedTxGraph;
use kyoto::node;
use kyoto::node::node_messages::NodeMessage;
use kyoto::prelude::bitcoin::BlockHash as KyotoBlockHash;
use kyoto::prelude::bitcoin::Transaction as KyotoTransaction;

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

    /// Into `Client`.
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
// Note: kyoto Node doesn't impl Debug
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
        println!("Syncing..");
        
        let (mut sender, receiver) = self.inner.split();

        while let Some(message) = receiver.recv().await {
            match message {
                NodeMessage::Block(_) => {
                    // note: we need the block height from this event
                    // in order to use `apply_block_relevant`
                }
                NodeMessage::Transaction(tx) => {
                    if let Some(height) = tx.height {
                        self.blocks.insert(height as u32, convert_hash(&tx.hash));
                    } else {
                        println!("Warning: missing block height for matching tx");
                    }
                    println!("Inserting tx");
                    let _ = self.graph.insert_tx(convert_tx(&tx.transaction));

                    // TODO: insert graph anchors?
                    //let _ = self.graph.insert_anchor();
                }
                NodeMessage::Synced(tip) => {
                    // TODO: try using `CheckPoint::insert`

                    self.blocks
                        .insert(tip.height as u32, convert_hash(&tip.hash));
                    println!("Synced");
                    sender.shutdown().await.unwrap();
                    break;
                }
                NodeMessage::Dialog(_) => {}
                NodeMessage::Warning(_) => {}
            }
        }

        self.as_update()
    }

    /// As `Update`.
    fn as_update(&mut self) -> Option<Update<K>> {
        let blocks: HashSet<BlockId> = self.blocks.iter().map(BlockId::from).collect();
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

    /// Shutdown
    pub fn shutdown(&mut self) { todo!() }
}

/// Update.
#[derive(Debug)]
pub struct Update<K> {
    /// `CheckPoint`
    pub cp: bdk_chain::local_chain::CheckPoint,
    /// `IndexedTxGraph`
    pub indexed_tx_graph:
        bdk_chain::IndexedTxGraph<ConfirmationTimeHeightAnchor, KeychainTxOutIndex<K>>,
}

/// Convert hash.
fn convert_hash(hash: &KyotoBlockHash) -> BlockHash {
    let s = hash.to_string();
    BlockHash::from_str(&s).unwrap()
}

/// Convert tx.
fn convert_tx(tx: &KyotoTransaction) -> Transaction {
    use bdk_chain::bitcoin::consensus::deserialize;
    use kyoto::prelude::bitcoin::consensus::serialize;
    let data = serialize(tx);
    deserialize(&data).expect("deserialize Transaction")
}
