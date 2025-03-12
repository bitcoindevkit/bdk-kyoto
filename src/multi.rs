//! Types for requesting updates for multiple wallets.
use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
};

use bdk_wallet::{
    chain::{
        keychain_txout::KeychainTxOutIndex, local_chain, local_chain::LocalChain,
        ConfirmationBlockTime, IndexedTxGraph, TxUpdate,
    },
    KeychainKind, Update, Wallet,
};
use kyoto::{BlockHash, Event, SyncUpdate, UnboundedReceiver};

use crate::ScanType;

/// A request when building a node for multiple wallets.
pub struct MultiSyncRequest<'a, H: Hash + Eq + Clone + Copy> {
    /// The unique identifier for a wallet. Typically a simple index like an integer.
    pub index: H,
    /// The scanning policy for this wallet.
    pub scan_type: ScanType,
    /// The wallet to fetch an update for.
    pub wallet: &'a Wallet,
}

/// Construct updates for multiple wallets.
pub struct MultiUpdateSubscriber<H: Hash + Eq + Clone + Copy> {
    pub(crate) receiver: UnboundedReceiver<Event>,
    pub(crate) wallet_map: HashMap<
        H,
        (
            LocalChain,
            IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<KeychainKind>>,
        ),
    >,
    pub(crate) chain_changeset: BTreeMap<u32, Option<BlockHash>>,
}

impl<H> MultiUpdateSubscriber<H>
where
    H: Hash + Eq + Clone + Copy,
{
    /// Return updates for all registered wallets for every time the light client
    /// syncs to connected peers.
    pub async fn sync(&mut self) -> impl Iterator<Item = (H, Update)> {
        while let Some(event) = self.receiver.recv().await {
            match event {
                Event::Block(indexed_block) => {
                    let hash = indexed_block.block.block_hash();
                    self.chain_changeset
                        .insert(indexed_block.height, Some(hash));
                    for (_, graph) in self.wallet_map.values_mut() {
                        let _ =
                            graph.apply_block_relevant(&indexed_block.block, indexed_block.height);
                    }
                }
                Event::BlocksDisconnected(disconnected) => {
                    for header in disconnected {
                        let height = header.height;
                        self.chain_changeset.insert(height, None);
                    }
                }
                Event::Synced(SyncUpdate {
                    tip: _,
                    recent_history,
                }) => {
                    recent_history.into_iter().for_each(|(height, header)| {
                        self.chain_changeset
                            .insert(height, Some(header.block_hash()));
                    });
                    break;
                }
            }
        }
        let mut responses = Vec::new();
        for (index, (local_chain, graph)) in &mut self.wallet_map {
            let tx_update = TxUpdate::from(graph.graph().clone());
            let last_active_indices = graph.index.last_used_indices();
            local_chain
                .apply_changeset(&local_chain::ChangeSet::from(self.chain_changeset.clone()))
                .expect("chain was initialized with genesis");
            let update = Update {
                tx_update,
                last_active_indices,
                chain: Some(local_chain.tip()),
            };
            responses.push((*index, update));
        }
        self.chain_changeset = BTreeMap::new();
        responses.into_iter()
    }
}
