//! BDK-Kyoto is an extension of [Kyoto](https://github.com/rustaceanrob/kyoto), a client-side implementation of BIP157/BIP158.
//! These proposals define a way for users to fetch transactions privately, using _compact block filters_.
//! You may want to read the specification [here](https://github.com/bitcoin/bips/blob/master/bip-0158.mediawiki).
//!
//! Kyoto runs as a psuedo-node, sending messages over the Bitcoin peer-to-peer layer, finding new peers to connect to, and managing a
//! light-weight database of Bitcoin block headers. As such, developing a wallet application using this crate is distinct from a typical
//! client/server relationship. Esplora and Electrum offer _proactive_ APIs, in that the servers will respond to events as they are requested.
//! In the case of running a node as a background process, the developer experience is far more _reactive_, in that the node may emit any number of events,
//! and the application may respond to them.
//!
//! BDK-Kyoto curates these events into structures that are easily handled by BDK APIs, making integration of compact block filters easily understood.
//! Developers are free to use [`bdk_wallet`], or only primatives found in [`bdk_core`](https://docs.rs/bdk_core/latest/bdk_core/) and [`bdk_chain`].
//!
//! ## Examples
//!
//! If you have an existing project that leverages `bdk_wallet`, building the compact block filter _node_ and _client_ is simple.
//! You may construct and configure a node to integrate with your wallet by using a [`LightClientBuilder`](crate).
//!
//! ```no_run
//! # const RECEIVE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
//! # const CHANGE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
//! use bdk_wallet::Wallet;
//! use bdk_wallet::bitcoin::Network;
//! use bdk_kyoto::builder::LightClientBuilder;
//! use bdk_kyoto::logger::PrintLogger;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut wallet = Wallet::create(RECEIVE, CHANGE)
//!         .network(Network::Signet)
//!         .create_wallet_no_persist()?;
//!
//!     let (mut node, mut client) = LightClientBuilder::new(&wallet).build()?;
//!
//!     tokio::task::spawn(async move { node.run().await });
//!
//!     let logger = PrintLogger::new();
//!     loop {
//!         if let Some(update) = client.update(&logger).await {
//!             wallet.apply_update(update)?;
//!             return Ok(());
//!         }
//!     }
//! }
//! ```
//!
//! Custom wallet implementations may still take advantage of BDK-Kyoto, however building the [`Client`] will involve configuring Kyoto directly.
//!
//! ```no_run
//! # const RECEIVE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
//! # const CHANGE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
//! # use std::collections::HashSet;
//! # use std::net::{IpAddr, Ipv4Addr};
//! # use std::str::FromStr;
//! # use bdk_wallet::bitcoin::{
//! #    constants::genesis_block, secp256k1::Secp256k1, Address, BlockHash, Network, ScriptBuf,
//! # };
//! # use bdk_wallet::chain::{
//! #    keychain_txout::KeychainTxOutIndex, local_chain::LocalChain, miniscript::Descriptor, FullTxOut,
//! #    IndexedTxGraph, SpkIterator, Merge,
//! # };
//! use bdk_kyoto::{Client, TrustedPeer, NodeBuilder, HeaderCheckpoint};
//! use bdk_kyoto::logger::PrintLogger;
//!
//! const TARGET_INDEX: u32 = 20;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let secp = Secp256k1::new();
//!     let (descriptor, _) = Descriptor::parse_descriptor(&secp, &RECEIVE)?;
//!     let (change_descriptor, _) = Descriptor::parse_descriptor(&secp, &CHANGE)?;
//!
//!     let g = genesis_block(Network::Signet).block_hash();
//!     let (mut chain, _) = LocalChain::from_genesis_hash(g);
//!
//!     let mut graph = IndexedTxGraph::new({
//!         let mut index = KeychainTxOutIndex::default();
//!         let _ = index.insert_descriptor(0usize, descriptor);
//!         let _ = index.insert_descriptor(1, change_descriptor);
//!         index
//!     });
//!
//!     let mut spks_to_watch: HashSet<ScriptBuf> = HashSet::new();
//!     for (_k, desc) in graph.index.keychains() {
//!         for (_i, spk) in SpkIterator::new_with_range(desc, 0..TARGET_INDEX) {
//!             spks_to_watch.insert(spk);
//!         }
//!     }
//!
//!     let peer = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
//!     let trusted = TrustedPeer::from_ip(peer);
//!     let peers = vec![trusted];
//!
//!     let builder = NodeBuilder::new(Network::Signet);
//!     let (mut node, kyoto_client) = builder
//!         .add_peers(peers)
//!         .add_scripts(spks_to_watch)
//!         .anchor_checkpoint(HeaderCheckpoint::new(
//!             170_000,
//!             BlockHash::from_str(
//!                 "00000041c812a89f084f633e4cf47e819a2f6b1c0a15162355a930410522c99d",
//!             ).unwrap(),
//!         ))
//!         .num_required_peers(2)
//!         .build_node()
//!         .unwrap();
//!
//!     let mut client = Client::from_index(chain.tip(), &graph.index, kyoto_client).unwrap();
//!
//!     tokio::task::spawn(async move { node.run().await });
//!
//!     let logger = PrintLogger::new();
//!     if let Some(update) = client.update(&logger).await {
//!         let _ = chain.apply_update(update.chain_update.unwrap())?;
//!         let mut indexed_tx_graph_changeset = graph.apply_update(update.tx_update);
//!         let index_changeset = graph
//!             .index
//!             .reveal_to_target_multi(&update.last_active_indices);
//!         indexed_tx_graph_changeset.merge(index_changeset.into());
//!         let _ = graph.apply_changeset(indexed_tx_graph_changeset);
//!     }
//!     client.shutdown().await?;
//!     Ok(())
//!}
//! ```

#![warn(missing_docs)]

use core::fmt;
use std::collections::BTreeMap;

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
    ClientError, DatabaseError, HeaderCheckpoint, Node, NodeBuilder, NodeMessage, NodeState,
    Receiver, ScriptBuf, SyncUpdate, Transaction, TrustedPeer, TxBroadcastPolicy, Txid, Warning,
    MAINNET_HEADER_CP, SIGNET_HEADER_CP,
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
    pub async fn broadcast(&self, tx: Transaction, policy: TxBroadcastPolicy) -> Result<(), Error> {
        self.sender
            .broadcast_tx(TxBroadcast {
                tx,
                broadcast_policy: policy,
            })
            .await
            .map_err(Error::from)
    }

    /// Add more scripts to the node. Could this just check a SPK index?
    pub async fn add_script(&self, script: impl Into<ScriptBuf>) -> Result<(), Error> {
        self.sender.add_script(script).await.map_err(Error::from)
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
