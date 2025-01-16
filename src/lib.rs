#![doc = include_str!("../README.md")]
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
//!     let LightClient { sender, mut receiver, node } = LightClientBuilder::new().build(&wallet)?;
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
//!     let LightClient { sender, mut receiver, node } = LightClientBuilder::new().build(&wallet)?;
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

#![warn(missing_docs)]
use core::fmt;
use core::{future::Future, pin::Pin};
use std::collections::BTreeMap;
use std::collections::HashSet;

type FutureResult<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

use bdk_wallet::chain::{
    keychain_txout::KeychainTxOutIndex,
    local_chain::{self, CheckPoint, LocalChain},
    spk_client::FullScanResponse,
    IndexedTxGraph,
};
use bdk_wallet::chain::{ConfirmationBlockTime, TxUpdate};

pub use bdk_wallet::chain::bitcoin::FeeRate;
pub use bdk_wallet::chain::local_chain::MissingGenesisError;

pub extern crate kyoto;

use bdk_wallet::KeychainKind;

pub use kyoto::core::builder::NodeDefault;
#[cfg(feature = "events")]
pub use kyoto::{DisconnectedHeader, FailurePayload};

pub use kyoto::ClientSender as EventSender;
use kyoto::{IndexedBlock, NodeMessage, RejectReason};
pub use kyoto::{
    NodeState, Receiver, ScriptBuf, SyncUpdate, TxBroadcast, TxBroadcastPolicy, Txid, Warning,
};

pub mod builder;
#[cfg(feature = "callbacks")]
pub mod logger;

#[derive(Debug)]
/// A node and associated structs to send and receive events to and from the node.
pub struct LightClient {
    /// Send events to a running node (i.e. broadcast a transaction).
    pub sender: EventSender,
    /// Receive wallet updates from a node.
    pub receiver: EventReceiver,
    /// The underlying node that must be run to fetch blocks from peers.
    pub node: NodeDefault,
}

/// Interpret events from a node that is running to apply
/// updates to an underlying wallet.
#[derive(Debug)]
pub struct EventReceiver {
    // channel receiver
    receiver: kyoto::Receiver<NodeMessage>,
    // changes to local chain
    chain: local_chain::LocalChain,
    // receive graph
    graph: IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<KeychainKind>>,
    // the network minimum to broadcast a transaction
    min_broadcast_fee: FeeRate,
}

impl EventReceiver {
    /// Build a light client event handler from a [`KeychainTxOutIndex`] and [`CheckPoint`].
    pub(crate) fn from_index(
        cp: CheckPoint,
        index: KeychainTxOutIndex<KeychainKind>,
        receiver: Receiver<NodeMessage>,
    ) -> Result<Self, MissingGenesisError> {
        Ok(Self {
            receiver,
            chain: LocalChain::from_tip(cp)?,
            graph: IndexedTxGraph::new(index.clone()),
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
    pub async fn update(
        &mut self,
        logger: &dyn NodeEventHandler,
    ) -> Option<FullScanResponse<KeychainKind>> {
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
    fn get_scan_response(&mut self) -> FullScanResponse<KeychainKind> {
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
    pub async fn next_event(&mut self, log_level: LogLevel) -> Option<Event> {
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
pub enum Event {
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
    ScanResponse(FullScanResponse<KeychainKind>),
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

/// Extend the functionality of [`Wallet`](bdk_wallet) for interoperablility
/// with the light client.
pub trait WalletExt {
    /// Collect relevant scripts for addition to the node. Peeks scripts
    /// `lookahead` + `last_revealed_index` for each keychain.
    fn peek_revealed_plus_lookahead(&self) -> Box<dyn Iterator<Item = ScriptBuf>>;
}

impl WalletExt for bdk_wallet::Wallet {
    fn peek_revealed_plus_lookahead(&self) -> Box<dyn Iterator<Item = ScriptBuf>> {
        let mut spks: HashSet<ScriptBuf> = HashSet::new();
        for keychain in [KeychainKind::External, KeychainKind::Internal] {
            let last_revealed = self.spk_index().last_revealed_index(keychain).unwrap_or(0);
            let lookahead_index = last_revealed + self.spk_index().lookahead();
            for index in 0..=lookahead_index {
                spks.insert(self.peek_address(keychain, index).script_pubkey());
            }
        }
        Box::new(spks.into_iter())
    }
}

/// Extend the [`EventSender`] functionality to work conveniently with a [`Wallet`](bdk_wallet).
pub trait EventSenderExt {
    /// Add all revealed scripts to the node to monitor.
    fn add_revealed_scripts<'a>(
        &'a self,
        wallet: &'a bdk_wallet::Wallet,
    ) -> FutureResult<'a, (), kyoto::ClientError>;
}

impl EventSenderExt for EventSender {
    fn add_revealed_scripts<'a>(
        &'a self,
        wallet: &'a bdk_wallet::Wallet,
    ) -> FutureResult<'a, (), kyoto::ClientError> {
        async fn _add_revealed(
            sender: &EventSender,
            wallet: &bdk_wallet::Wallet,
        ) -> Result<(), kyoto::ClientError> {
            for keychain in [KeychainKind::External, KeychainKind::Internal] {
                let scripts = wallet.spk_index().revealed_keychain_spks(keychain);
                for (_, script) in scripts {
                    sender.add_script(script).await?;
                }
            }
            Ok(())
        }
        Box::pin(_add_revealed(self, wallet))
    }
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
