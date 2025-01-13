//! Build an event receiver for multiple [`Wallet`](bdk_wallet).
use core::fmt;
use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
};

use bdk_chain::{
    bitcoin::FeeRate,
    keychain_txout::KeychainTxOutIndex,
    local_chain::{LocalChain, MissingGenesisError},
    spk_client::FullScanResponse,
    CheckPoint, ConfirmationBlockTime, IndexedTxGraph, TxUpdate,
};
use kyoto::{IndexedBlock, NodeMessage, Receiver, SyncUpdate};

use crate::NodeEventHandler;
use crate::StringExt;

/// One of potentially multiple sync requets for the [`MultiEventReceiver`]
/// to handle.
#[derive(Debug)]
pub struct MultiSyncRequest<H: Hash + Eq + Clone + Copy, K: fmt::Debug + Clone + Ord> {
    /// A unique index to identify the [`Wallet`](bdk_wallet).
    pub index: H,
    /// The tip of the chain for this wallet.
    pub checkpoint: CheckPoint,
    /// The script pubkeys for this wallet.
    pub spk_index: KeychainTxOutIndex<K>,
}

/// Interpret events from a node that is running to apply
/// multiple wallets in parallel.
#[derive(Debug)]
pub struct MultiEventReceiver<H, K> {
    // channel receiver
    receiver: kyoto::Receiver<NodeMessage>,
    // map of chain and spk index to an index
    map: HashMap<
        H,
        (
            LocalChain,
            IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<K>>,
        ),
    >,
    // the network minimum to broadcast a transaction
    min_broadcast_fee: FeeRate,
}

impl<H, K> MultiEventReceiver<H, K>
where
    H: Hash + Eq + Clone + Copy,
    K: fmt::Debug + Clone + Ord,
{
    /// Build a light client event handler from a [`KeychainTxOutIndex`] and [`CheckPoint`].
    pub fn from_requests(
        requests: impl IntoIterator<Item = MultiSyncRequest<H, K>>,
        receiver: Receiver<NodeMessage>,
    ) -> Result<Self, MissingGenesisError> {
        let mut map = HashMap::new();
        for MultiSyncRequest {
            index,
            checkpoint,
            spk_index,
        } in requests
        {
            map.insert(
                index,
                (
                    LocalChain::from_tip(checkpoint)?,
                    IndexedTxGraph::new(spk_index),
                ),
            );
        }
        Ok(Self {
            receiver,
            map,
            min_broadcast_fee: FeeRate::BROADCAST_MIN,
        })
    }

    /// Return the most recent update from the node once it has synced to the network's tip.
    /// This may take a significant portion of time during wallet recoveries or dormant wallets.
    /// Note that you may call this method in a loop as long as the node is running.
    ///
    /// A reference to a [`NodeEventHandler`] is required, which handles events emitted from a
    /// running node. Production applications should define how the application handles
    /// these events and displays them to end users.
    #[cfg(feature = "callbacks")]
    pub async fn updates(
        &mut self,
        logger: &dyn NodeEventHandler,
    ) -> impl Iterator<Item = (H, FullScanResponse<K>)> {
        use bdk_chain::local_chain;

        let mut chain_changeset = BTreeMap::new();
        while let Ok(message) = self.receiver.recv().await {
            self.log(&message, logger);
            match message {
                NodeMessage::Block(IndexedBlock { height, block }) => {
                    let hash = block.header.block_hash();
                    chain_changeset.insert(height, Some(hash));
                    for (_, graph) in self.map.values_mut() {
                        let _ = graph.apply_block_relevant(&block, height);
                    }
                }
                NodeMessage::BlocksDisconnected(headers) => {
                    for header in headers {
                        let height = header.height;
                        chain_changeset.insert(height, None);
                    }
                }
                NodeMessage::Synced(SyncUpdate {
                    tip: _,
                    recent_history,
                }) => {
                    recent_history.into_iter().for_each(|(height, header)| {
                        chain_changeset.insert(height, Some(header.block_hash()));
                    });
                    break;
                }
                NodeMessage::FeeFilter(fee_filter) => {
                    if self.min_broadcast_fee < fee_filter {
                        self.min_broadcast_fee = fee_filter;
                    }
                }
                _ => (),
            }
        }
        let mut responses = Vec::new();
        for (index, (local_chain, graph)) in &mut self.map {
            let tx_update = TxUpdate::from(graph.graph().clone());
            let last_active_indices = graph.index.last_used_indices();
            local_chain
                .apply_changeset(&local_chain::ChangeSet::from(chain_changeset.clone()))
                .expect("chain was initialized with genesis");
            let update = FullScanResponse {
                tx_update,
                last_active_indices,
                chain_update: Some(local_chain.tip()),
            };
            responses.push((*index, update));
        }
        responses.into_iter()
    }

    #[cfg(feature = "callbacks")]
    fn log(&self, message: &NodeMessage, logger: &dyn NodeEventHandler) {
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
                logger.tx_sent(*t);
            }
            NodeMessage::TxBroadcastFailure(r) => {
                logger.tx_failed(r.txid, r.reason.map(|reason| reason.into_string()))
            }
            NodeMessage::ConnectionsMet => logger.connections_met(),
            _ => (),
        }
    }
}
