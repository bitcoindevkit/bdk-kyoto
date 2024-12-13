//! BDK-Kyoto is an extension of [Kyoto](https://github.com/rustaceanrob/kyoto), a client-side implementation of BIP157/BIP158.
//! These proposals define a way for users to fetch transactions privately, using _compact block
//! filters_. You may want to read the specification [here](https://github.com/bitcoin/bips/blob/master/bip-0158.mediawiki).
//!
//! Kyoto runs as a psuedo-node, sending messages over the Bitcoin peer-to-peer layer, finding new
//! peers to connect to, and managing a light-weight database of Bitcoin block headers. As such,
//! developing a wallet application using this crate is distinct from a typical client/server
//! relationship. Esplora and Electrum offer _proactive_ APIs, in that the servers will respond to
//! events as they are requested. In the case of running a node as a background process, the
//! developer experience is far more _reactive_, in that the node may emit any number of events, and
//! the application may respond to them.
//!
//! BDK-Kyoto curates these events into structures that are easily handled by BDK APIs, making
//! integration of compact block filters easily understood. Developers are free to use [`bdk_wallet`], or only primatives found in [`bdk_core`](https://docs.rs/bdk_core/latest/bdk_core/) and [`bdk_chain`].
//!
//! ## Examples
//!
//! If you have an existing project that leverages `bdk_wallet`, building the compact block filter
//! _node_ and _client_ is simple. You may construct and configure a node to integrate with your
//! wallet by using a [`LightClientBuilder`](crate).
//!
//! ```no_run
//! # const RECEIVE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
//! # const CHANGE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
//! use bdk_wallet::Wallet;
//! use bdk_wallet::bitcoin::Network;
//! use bdk_kyoto::builder::LightClientBuilder;
//! use bdk_kyoto::logger::PrintLogger;
//! use bdk_kyoto::LightClient;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut wallet = Wallet::create(RECEIVE, CHANGE)
//!         .network(Network::Signet)
//!         .create_wallet_no_persist()?;
//!
//!     let LightClient { sender, mut receiver, node } = LightClientBuilder::new(&wallet).build()?;
//!
//!     tokio::task::spawn(async move { node.run().await });
//!
//!     let logger = PrintLogger::new();
//!     loop {
//!         if let Some(update) = receiver.update(&logger).await {
//!             wallet.apply_update(update)?;
//!             return Ok(());
//!         }
//!     }
//! }
//! ```
//!
//! It may be preferable to use events instead of defining a trait. To do so,
//! the workflow for building the node remains the same.
//!
//! ```no_run
//! # const RECEIVE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
//! # const CHANGE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
//! # use bdk_kyoto::builder::LightClientBuilder;
//! # use bdk_kyoto::{Event, LogLevel, LightClient};
//! # use bdk_wallet::bitcoin::Network;
//! # use bdk_wallet::Wallet;
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut wallet = Wallet::create(RECEIVE, CHANGE)
//!         .network(Network::Signet)
//!         .create_wallet_no_persist()?;
//!
//!     let LightClient { sender, mut receiver, node } = LightClientBuilder::new(&wallet).build()?;
//!
//!     tokio::task::spawn(async move { node.run().await });
//!
//!     loop {
//!         if let Some(event) = receiver.next_event(LogLevel::Info).await {
//!             match event {
//!                 Event::ScanResponse(full_scan_result) => {
//!                     wallet.apply_update(full_scan_result).unwrap();
//!                 },
//!                 _ => (),
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! Custom wallet implementations may still take advantage of BDK-Kyoto, however building the
//! [`EventReceiver`] will involve configuring Kyoto directly.
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
//! use bdk_kyoto::EventReceiver;
//! use bdk_kyoto::logger::PrintLogger;
//! use bdk_kyoto::kyoto::{TrustedPeer, NodeBuilder, HeaderCheckpoint};
//!
//! const TARGET_INDEX: u32 = 20;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let secp = Secp256k1::new();
//!     let (descriptor, _) = Descriptor::parse_descriptor(&secp, &RECEIVE)?;
//!     let (change_descriptor, _) = Descriptor::parse_descriptor(&secp, &CHANGE)?;
//!
//!     let genesis_hash = genesis_block(Network::Signet).block_hash();
//!     let (mut chain, _) = LocalChain::from_genesis_hash(genesis_hash);
//!
//!     let mut graph = IndexedTxGraph::new({
//!         let mut index = KeychainTxOutIndex::default();
//!         let _ = index.insert_descriptor("external", descriptor);
//!         let _ = index.insert_descriptor("internal", change_descriptor);
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
//!     let cp = HeaderCheckpoint::closest_checkpoint_below_height(170_000, Network::Signet);
//!
//!     let builder = NodeBuilder::new(Network::Signet);
//!     let (node, kyoto_client) = builder
//!         .add_peer(trusted)
//!         .add_scripts(spks_to_watch)
//!         .anchor_checkpoint(cp)
//!         .num_required_peers(2)
//!         .build_node()
//!         .unwrap();
//!
//!     let (sender, receiver) = kyoto_client.split();
//!     let mut client = EventReceiver::from_index(Network::Signet, &graph.index, receiver);
//!
//!     tokio::task::spawn(async move { node.run().await });
//!
//!     let logger = PrintLogger::new();
//!     if let Some(update) = client.update(&logger).await {
//!         let _ = chain.apply_update(update.chain_update.unwrap())?;
//!         let _ = graph.apply_update(update.tx_update);
//!         let _ = graph
//!             .index
//!             .reveal_to_target_multi(&update.last_active_indices);
//!     }
//!     sender.shutdown().await?;
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]
use core::fmt;
use std::collections::BTreeMap;

use bdk_chain::{
    bitcoin::{constants::genesis_block, params::Params},
    keychain_txout::KeychainTxOutIndex,
    local_chain::{self, LocalChain},
    spk_client::FullScanResponse,
    IndexedTxGraph,
};
use bdk_chain::{ConfirmationBlockTime, TxUpdate};

pub use bdk_chain::bitcoin::FeeRate;
pub use bdk_chain::local_chain::MissingGenesisError;

pub extern crate kyoto;

#[cfg(feature = "wallet")]
use bdk_wallet::KeychainKind;
#[cfg(feature = "rusqlite")]
pub use kyoto::core::builder::NodeDefault;
#[cfg(feature = "events")]
pub use kyoto::{DisconnectedHeader, FailurePayload};

pub use kyoto::ClientSender as EventSender;
use kyoto::{IndexedBlock, NodeMessage, RejectReason};
pub use kyoto::{NodeState, Receiver, SyncUpdate, TxBroadcast, TxBroadcastPolicy, Txid, Warning};

#[cfg(all(feature = "wallet", feature = "rusqlite"))]
pub mod builder;
#[cfg(feature = "callbacks")]
pub mod logger;

#[cfg(feature = "wallet")]
#[derive(Debug)]
/// A node and associated structs to send and receive events to and from the node.
pub struct LightClient {
    /// Send events to a running node (i.e. broadcast a transaction).
    pub sender: EventSender,
    /// Receive wallet updates from a node.
    pub receiver: EventReceiver<KeychainKind>,
    /// The underlying node that must be run to fetch blocks from peers.
    pub node: NodeDefault,
}

/// Interpret events from a node that is running to apply
/// updates to an underlying wallet.
#[derive(Debug)]
pub struct EventReceiver<K> {
    // channel receiver
    receiver: kyoto::Receiver<NodeMessage>,
    // changes to local chain
    chain: local_chain::LocalChain,
    // receive graph
    graph: IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<K>>,
    // the network minimum to broadcast a transaction
    min_broadcast_fee: FeeRate,
}

impl<K> EventReceiver<K>
where
    K: fmt::Debug + Clone + Ord,
{
    /// Build a light client event handler from a [`KeychainTxOutIndex`] and [`CheckPoint`].
    pub fn from_index(
        params: impl AsRef<Params>,
        index: &KeychainTxOutIndex<K>,
        receiver: Receiver<NodeMessage>,
    ) -> Self {
        let (chain, _) = LocalChain::from_genesis_hash(genesis_block(params).block_hash());
        Self {
            receiver,
            chain,
            graph: IndexedTxGraph::new(index.clone()),
            min_broadcast_fee: FeeRate::BROADCAST_MIN,
        }
    }

    /// Return the most recent update from the node once it has synced to the network's tip.
    /// This may take a significant portion of time during wallet recoveries or dormant wallets.
    /// Note that you may call this method in a loop as long as the node is running.
    ///
    /// A reference to a [`NodeEventHandler`] is required, which handles events emitted from a
    /// running node. Production applications should define how the application handles
    /// these events and displays them to end users.
    #[cfg(feature = "callbacks")]
    pub async fn update(&mut self, logger: &dyn NodeEventHandler) -> Option<FullScanResponse<K>> {
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
                NodeMessage::FeeFilter(fee_filter) => {
                    if self.min_broadcast_fee < fee_filter {
                        self.min_broadcast_fee = fee_filter;
                    }
                }
                _ => (),
            }
        }
        self.chain
            .apply_changeset(&local_chain::ChangeSet::from(chain_changeset))
            .expect("chain was initialized with genesis");
        Some(self.get_scan_response())
    }

    // Send dialogs to an arbitrary logger
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

    // When the client is believed to have synced to the chain tip of most work,
    // we can return a wallet update.
    fn get_scan_response(&mut self) -> FullScanResponse<K> {
        let tx_update = TxUpdate::from(self.graph.graph().clone());
        let graph = core::mem::take(&mut self.graph);
        let last_active_indices = graph.index.last_used_indices();
        self.graph = IndexedTxGraph::new(graph.index);
        FullScanResponse {
            tx_update,
            last_active_indices,
            chain_update: Some(self.chain.tip()),
        }
    }

    /// Wait for the next event from the client. If no event is ready,
    /// `None` will be returned. Otherwise, the event will be `Some(..)`.
    ///
    /// Blocks will be processed while waiting for the next event of relevance.
    /// When the node is fully synced to the chain of all connected peers,
    /// an update for the provided keychain or underlying wallet will be returned.
    ///
    /// Informational messages on the node operation may be filtered out with
    /// [`LogLevel::Warning`], which will only emit warnings when called.
    #[cfg(feature = "events")]
    pub async fn next_event(&mut self, log_level: LogLevel) -> Option<Event<K>> {
        while let Ok(message) = self.receiver.recv().await {
            match message {
                NodeMessage::Dialog(log) => {
                    if matches!(log_level, LogLevel::Info) {
                        return Some(Event::Log(log));
                    }
                }
                NodeMessage::Warning(warning) => return Some(Event::Warning(warning)),
                NodeMessage::StateChange(node_state) => {
                    return Some(Event::StateChange(node_state))
                }
                NodeMessage::ConnectionsMet => return Some(Event::PeersFound),
                NodeMessage::Block(IndexedBlock { height, block }) => {
                    // This is weird but I'm having problems doing things differently.
                    let mut chain_changeset = BTreeMap::new();
                    chain_changeset.insert(height, Some(block.block_hash()));
                    self.chain
                        .apply_changeset(&local_chain::ChangeSet::from(chain_changeset))
                        .expect("chain initialized with genesis");
                    let _ = self.graph.apply_block_relevant(&block, height);
                    if matches!(log_level, LogLevel::Info) {
                        return Some(Event::Log(format!(
                            "Applied block {} to keychain",
                            block.block_hash()
                        )));
                    }
                }
                NodeMessage::Synced(SyncUpdate {
                    tip: _,
                    recent_history,
                }) => {
                    let mut chain_changeset = BTreeMap::new();
                    recent_history.into_iter().for_each(|(height, header)| {
                        chain_changeset.insert(height, Some(header.block_hash()));
                    });
                    self.chain
                        .apply_changeset(&local_chain::ChangeSet::from(chain_changeset))
                        .expect("chain was initialized with genesis");
                    let result = self.get_scan_response();
                    return Some(Event::ScanResponse(result));
                }
                NodeMessage::BlocksDisconnected(headers) => {
                    let mut chain_changeset = BTreeMap::new();
                    for dc in &headers {
                        let height = dc.height;
                        chain_changeset.insert(height, None);
                    }
                    self.chain
                        .apply_changeset(&local_chain::ChangeSet::from(chain_changeset))
                        .expect("chain was initialized with genesis.");
                    return Some(Event::BlocksDisconnected(headers));
                }
                NodeMessage::TxSent(txid) => {
                    return Some(Event::TxSent(txid));
                }
                NodeMessage::TxBroadcastFailure(failure_payload) => {
                    return Some(Event::TxFailed(failure_payload));
                }
                NodeMessage::FeeFilter(fee_filter) => {
                    if self.min_broadcast_fee < fee_filter {
                        self.min_broadcast_fee = fee_filter;
                    }
                }
                _ => continue,
            }
        }
        None
    }

    /// The minimum fee required for a transaction to propagate to the connected peers.
    pub fn broadcast_minimum(&self) -> FeeRate {
        self.min_broadcast_fee
    }
}

/// Handle dialog and state changes from a node with some arbitrary behavior.
/// The primary purpose of this trait is not to respond to events by persisting changes,
/// or acting on the underlying wallet. Instead, this trait should be used to drive changes in user
/// interface behavior or keep a simple log. Relevant events that effect on the wallet are handled
/// automatically in [`EventReceiver::update`](EventReceiver).
#[cfg(feature = "callbacks")]
pub trait NodeEventHandler: Send + Sync + fmt::Debug + 'static {
    /// Make use of some message the node has sent.
    fn dialog(&self, dialog: String);
    /// Make use of some warning the node has sent.
    fn warning(&self, warning: Warning);
    /// Handle a change in the node's state.
    fn state_changed(&self, state: NodeState);
    /// The required number of connections for the node was met.
    fn connections_met(&self);
    /// A transaction was broadcast to at least one peer.
    fn tx_sent(&self, txid: Txid);
    /// A transaction was rejected or failed to broadcast.
    fn tx_failed(&self, txid: Txid, reject_reason: Option<String>);
    /// A list of block heights were reorganized
    fn blocks_disconnected(&self, blocks: Vec<u32>);
    /// The node has synced to the height of the connected peers.
    fn synced(&self, tip: u32);
}

/// Events emitted by a node that may be used by a wallet or application.
#[cfg(feature = "events")]
pub enum Event<K: fmt::Debug + Clone + Ord> {
    /// Information about the current node process.
    Log(String),
    /// Warnings emitted by the node that may effect sync times or node operation.
    Warning(Warning),
    /// All required connnections have been met.
    PeersFound,
    /// A transaction was broadcast.
    TxSent(Txid),
    /// A transaction failed to broadcast or was rejected.
    TxFailed(FailurePayload),
    /// The node is performing a new task.
    StateChange(NodeState),
    /// A result after scanning compact block filters to the tip of the chain.
    ///
    /// ## Note
    ///
    /// This event will be emitted every time a new block is found while the node
    /// is running and is connected to peers.
    ScanResponse(FullScanResponse<K>),
    /// Blocks were reorganized from the chain of most work.
    ///
    /// ## Note
    ///
    /// No action is required from the developer, as these events are already
    /// handled within the [`EventReceiver`]. This event is to inform the user of
    /// such an event.
    BlocksDisconnected(Vec<DisconnectedHeader>),
}

/// Filter [`Event`] by a specified level. [`LogLevel::Info`] will pass
/// through both [`Event::Log`] and [`Event::Warning`]. [`LogLevel::Warning`]
/// will omit [`Event::Log`] events.
#[cfg(feature = "events")]
pub enum LogLevel {
    /// Receive info messages and warnings.
    Info,
    /// Omit info messages and only receive warnings.
    Warning,
}

trait StringExt {
    fn into_string(self) -> String;
}

impl StringExt for RejectReason {
    fn into_string(self) -> String {
        let message = match self {
            RejectReason::Malformed => "Message could not be decoded.",
            RejectReason::Invalid => "Transaction was invalid for some reason.",
            RejectReason::Obsolete => "Client version is no longer supported.",
            RejectReason::Duplicate => "Duplicate version message received.",
            RejectReason::NonStandard => "Transaction was nonstandard.",
            RejectReason::Dust => "One or more outputs are below the dust threshold.",
            RejectReason::Fee => "Transaction does not have enough fee to be mined.",
            RejectReason::Checkpoint => "Inconsistent with compiled checkpoint.",
        };
        message.into()
    }
}
