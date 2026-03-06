#![doc = include_str!("../README.md")]
//! ## Examples
//!
//! If you have an existing project that leverages `bdk_wallet`, building the compact block filter
//! _node_ and _client_ is simple. You may construct and configure a node to integrate with your
//! wallet by using the [`BuilderExt`](crate::builder) and [`Builder`](crate::builder).
//!
//! ```no_run
//! # const RECEIVE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
//! # const CHANGE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
//! use bdk_wallet::Wallet;
//! use bdk_wallet::bitcoin::Network;
//! use bdk_kyoto::builder::{Builder, BuilderExt};
//! use bdk_kyoto::{LightClient, ScanType};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut wallet = Wallet::create(RECEIVE, CHANGE)
//!         .network(Network::Signet)
//!         .create_wallet_no_persist()?;
//!
//!     let client = Builder::new(Network::Signet).build_with_wallet(&wallet, ScanType::Sync)?;
//!     let (client, _, mut update_subscriber) = client.subscribe();
//!     client.start();
//!
//!     loop {
//!         let update = update_subscriber.update().await?;
//!         wallet.apply_update(update)?;
//!         return Ok(());
//!     }
//! }
//! ```

#![warn(missing_docs)]
use std::collections::BTreeMap;
use std::collections::HashSet;

use bdk_wallet::chain::BlockId;
use bdk_wallet::chain::CheckPoint;
use bdk_wallet::chain::DescriptorId;
pub use bdk_wallet::Update;

use bdk_wallet::chain::{keychain_txout::KeychainTxOutIndex, IndexedTxGraph};
use bdk_wallet::chain::{ConfirmationBlockTime, TxUpdate};
use bdk_wallet::KeychainKind;

pub extern crate bip157;

use bip157::chain::BlockHeaderChanges;
use bip157::tokio;
use bip157::IndexedBlock;
use bip157::ScriptBuf;
#[doc(inline)]
pub use bip157::{
    BlockHash, ClientError, FeeRate, HeaderCheckpoint, Info, Node, RejectPayload, RejectReason,
    Requester, TrustedPeer, Warning, Wtxid,
};
use bip157::{Event, SyncUpdate};

#[doc(inline)]
pub use bip157::Receiver;
#[doc(inline)]
pub use bip157::UnboundedReceiver;

#[doc(inline)]
pub use builder::BuilderExt;
pub mod builder;

/// State of the light client.
pub mod state {
    /// Client state when idle.
    pub struct Idle;
    /// Client state when subscribed to events.
    pub struct Subscribed;
    /// Client state when active.
    pub struct Active;
}

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for state::Idle {}
impl sealed::Sealed for state::Subscribed {}
impl sealed::Sealed for state::Active {}

/// State of the client.
pub trait State: sealed::Sealed {}

impl State for state::Idle {}
impl State for state::Subscribed {}
impl State for state::Active {}

/// Subscribe to events, notably
#[derive(Debug)]
pub struct LoggingSubscribers {
    /// Receive informational messages as the node runs.
    pub info_subscriber: Receiver<Info>,
    /// Receive warnings from the node as it runs.
    pub warning_subscriber: UnboundedReceiver<Warning>,
}

/// A client and associated structs to send and receive events to and from a node process.
///
/// The client has three states:
/// - [`Idle`]: the client has been initialized.
/// - [`Subscribed`]: the application is ready to handle logs and updates, but the process is not
///   running yet
/// - [`Active`]: the client is actively fetching data and may now handle requests.
#[derive(Debug)]
pub struct LightClient<S: State, W: Wallets> {
    // Send events to a running node (i.e. broadcast a transaction).
    requester: Requester,
    // Receive info/warnings from the node as it runs.
    logging_subscribers: Option<LoggingSubscribers>,
    // Receive wallet updates from a node.
    update_subscriber: Option<UpdateSubscriber<W>>,
    // The underlying node that must be run to fetch blocks from peers.
    node: Option<Node>,
    _marker: core::marker::PhantomData<S>,
}

impl<W: Wallets> LightClient<state::Idle, W> {
    fn new(
        requester: Requester,
        logging: LoggingSubscribers,
        update: UpdateSubscriber<W>,
        node: bip157::Node,
    ) -> LightClient<state::Idle, W> {
        LightClient {
            requester,
            logging_subscribers: Some(logging),
            update_subscriber: Some(update),
            node: Some(node),
            _marker: core::marker::PhantomData,
        }
    }

    /// Subscribe to events emitted by the underlying data fetching process. This includes logging
    /// and wallet updates. During this step, one may start threads that log to a file and apply
    /// updates to a wallet.
    ///
    /// # Returns
    ///
    /// - [`LightClient<Subscribed>`], a client ready to start.
    /// - [`LoggingSubscribers`], info and warning messages to display to a user or write to file.
    /// - [`UpdateSubscriber`], used to await updates related to the user's wallet.
    pub fn subscribe(
        mut self,
    ) -> (
        LightClient<state::Subscribed, W>,
        LoggingSubscribers,
        UpdateSubscriber<W>,
    ) {
        let logging =
            core::mem::take(&mut self.logging_subscribers).expect("cannot subscribe twice.");
        let updates =
            core::mem::take(&mut self.update_subscriber).expect("cannot subscribe twice.");
        let client = LightClient {
            requester: self.requester,
            logging_subscribers: None,
            update_subscriber: None,
            node: self.node,
            _marker: core::marker::PhantomData,
        };
        (client, logging, updates)
    }
}

impl<W: Wallets> LightClient<state::Subscribed, W> {
    /// Start fetching data for the wallet on a dedicated [`tokio::task`]. This will continually
    /// run until terminated or no peers could be found.
    ///
    /// # Panics
    ///
    /// If there is no [`tokio::runtime::Runtime`] to drive execution. Common in synchronous
    /// setups.
    pub fn start(mut self) -> LightClient<state::Active, W> {
        let node = core::mem::take(&mut self.node).expect("cannot start twice.");
        tokio::task::spawn(async move { node.run().await });
        LightClient {
            requester: self.requester,
            logging_subscribers: None,
            update_subscriber: None,
            node: None,
            _marker: core::marker::PhantomData,
        }
    }

    /// Take the underlying node process to run in a custom way. Examples include using a dedicated
    /// [`tokio::runtime::Runtime`] or [`tokio::runtime::Handle`] to drive execution.
    pub fn managed_start(mut self) -> (LightClient<state::Active, W>, Node) {
        let node = core::mem::take(&mut self.node).expect("cannot start twice.");
        let client = LightClient {
            requester: self.requester,
            logging_subscribers: None,
            update_subscriber: None,
            node: None,
            _marker: core::marker::PhantomData,
        };
        (client, node)
    }
}

impl<W: Wallets> LightClient<state::Active, W> {
    /// The client is active and may now handle requests with a [`Requester`].
    pub fn requester(self) -> Requester {
        self.requester
    }
}

impl<W: Wallets> From<LightClient<state::Active, W>> for Requester {
    fn from(value: LightClient<state::Active, W>) -> Self {
        value.requester
    }
}

impl<W: Wallets> AsRef<Requester> for LightClient<state::Active, W> {
    fn as_ref(&self) -> &Requester {
        &self.requester
    }
}

/// Tag for number of wallets.
pub mod wallets {
    /// Client for a single wallet.
    #[derive(Debug)]
    pub struct Single;
    /// Client for multiple wallets.
    #[derive(Debug)]
    pub struct Multiple;
}

impl sealed::Sealed for wallets::Single {}
impl sealed::Sealed for wallets::Multiple {}

/// Number of wallets.
pub trait Wallets: sealed::Sealed {}

impl Wallets for wallets::Single {}
impl Wallets for wallets::Multiple {}

/// Interpret events from a node that is running to apply
/// updates to an underlying wallet.
#[derive(Debug)]
pub struct UpdateSubscriber<W: Wallets> {
    // request information from the client
    requester: Requester,
    // channel receiver
    receiver: UnboundedReceiver<Event>,
    // queued blocks to fetch
    queued_blocks: Vec<BlockHash>,
    // queued scripts to check filters
    spk_cache: HashSet<ScriptBuf>,
    // processes events for the wallet.
    single_update_builder: Option<UpdateBuilder>,
    // process events for multiple wallets.
    multiple_updates_builder: Option<BTreeMap<DescriptorId, UpdateBuilder>>,
    _marker: core::marker::PhantomData<W>,
}

impl<W: Wallets> UpdateSubscriber<W> {
    fn new(
        requester: Requester,
        scan_type: ScanType,
        receiver: UnboundedReceiver<Event>,
        cp: CheckPoint,
        graph: IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<KeychainKind>>,
    ) -> UpdateSubscriber<wallets::Single> {
        let update_builder = UpdateBuilder::new(cp, graph);
        let spk_cache = update_builder.peek_scripts_from_scantype(scan_type);
        UpdateSubscriber {
            requester,
            receiver,
            single_update_builder: Some(update_builder),
            multiple_updates_builder: None,
            queued_blocks: Vec::new(),
            spk_cache,
            _marker: core::marker::PhantomData,
        }
    }

    fn new_multiple(
        requester: Requester,
        receiver: UnboundedReceiver<Event>,
        wallet_iter: impl Iterator<
            Item = (
                DescriptorId,
                ScanType,
                CheckPoint,
                IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<KeychainKind>>,
            ),
        >,
    ) -> UpdateSubscriber<wallets::Multiple> {
        let mut update_map = BTreeMap::new();
        let mut spk_cache = HashSet::new();
        for wallet in wallet_iter {
            let update_builder = UpdateBuilder::new(wallet.2, wallet.3);
            spk_cache.extend(update_builder.peek_scripts_from_scantype(wallet.1));
            update_map.insert(wallet.0, update_builder);
        }
        UpdateSubscriber {
            requester,
            receiver,
            single_update_builder: None,
            multiple_updates_builder: Some(update_map),
            queued_blocks: Vec::new(),
            spk_cache,
            _marker: core::marker::PhantomData,
        }
    }

    async fn sync(&mut self) -> Result<(), UpdateError> {
        while let Some(message) = self.receiver.recv().await {
            match message {
                Event::IndexedFilter(filter) => {
                    let block_hash = filter.block_hash();
                    if filter.contains_any(self.spk_cache.iter()) {
                        self.queued_blocks.push(block_hash);
                    }
                }
                Event::ChainUpdate(changeset) => {
                    if let Some(single) = self.single_update_builder.as_mut() {
                        single.apply_chain_event(&changeset);
                    }
                    if let Some(multiple) = self.multiple_updates_builder.as_mut() {
                        for (_id, builder) in multiple.iter_mut() {
                            builder.apply_chain_event(&changeset);
                        }
                    }
                }
                Event::FiltersSynced(SyncUpdate {
                    tip: _,
                    recent_history: _,
                }) => {
                    for hash in core::mem::take(&mut self.queued_blocks) {
                        let block = self
                            .requester
                            .get_block(hash)
                            .await
                            .map_err(|_| UpdateError::NodeStopped)?;
                        if let Some(single) = self.single_update_builder.as_mut() {
                            single.apply_block_event(&block);
                        }
                        if let Some(multiple) = self.multiple_updates_builder.as_mut() {
                            for (_id, builder) in multiple.iter_mut() {
                                builder.apply_block_event(&block);
                            }
                        }
                    }
                    if let Some(single) = self.single_update_builder.as_mut() {
                        self.spk_cache
                            .extend(single.peek_script_to_keychain_lookahead());
                    }
                    if let Some(multiple) = self.multiple_updates_builder.as_mut() {
                        for (_id, builder) in multiple.iter() {
                            self.spk_cache
                                .extend(builder.peek_script_to_keychain_lookahead());
                        }
                    }
                    return Ok(());
                }
                _ => (),
            }
        }
        Err(UpdateError::NodeStopped)
    }
}

impl UpdateSubscriber<wallets::Single> {
    /// Return the most recent [`Update`] for a wallet once it has synced to the network's tip.
    /// This may take a significant portion of time during wallet recoveries or dormant wallets.
    /// Note that you may call this method in a loop as long as the node is running.
    ///
    /// **Warning**
    ///
    /// This method is _not_ cancel safe. You cannot use it within a `tokio::select` arm.
    pub async fn update(&mut self) -> Result<Update, UpdateError> {
        self.sync().await?;
        Ok(self.single_update_builder.as_mut().unwrap().finish())
    }
}

impl UpdateSubscriber<Multiple> {
    /// Return a set of [`Update`] for the configured wallets when synced to the network's tip. The
    /// [`Update`] are grouped with the [`DescriptorId`] of the external descriptor for each
    /// wallet.
    ///
    /// This may take a significant portion of time during wallet recoveries or dormant wallets.
    /// Note that you may call this method in a loop as long as the node is running.
    ///
    /// **Warning**
    ///
    /// This method is _not_ cancel safe. You cannot use it within a `tokio::select` arm.
    pub async fn updates(
        &mut self,
    ) -> Result<impl Iterator<Item = (DescriptorId, Update)>, UpdateError> {
        self.sync().await?;
        let mut map = BTreeMap::new();
        for (id, builder) in self.multiple_updates_builder.as_mut().unwrap().iter_mut() {
            map.insert(*id, builder.finish());
        }
        Ok(map.into_iter())
    }
}

#[derive(Debug)]
struct UpdateBuilder {
    // Changes to the wallet local chain.
    cp: CheckPoint,
    // Transaction graph, required to process incoming blocks.
    graph: IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<KeychainKind>>,
}

impl UpdateBuilder {
    fn new(
        cp: CheckPoint,
        graph: IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<KeychainKind>>,
    ) -> Self {
        Self { cp, graph }
    }

    fn apply_chain_event(&mut self, event: &BlockHeaderChanges) {
        match event {
            BlockHeaderChanges::Connected(at) => {
                let block_id = BlockId {
                    hash: at.block_hash(),
                    height: at.height,
                };
                self.cp = self.cp.clone().insert(block_id);
            }
            BlockHeaderChanges::Reorganized {
                accepted,
                reorganized: _,
            } => {
                for header in accepted {
                    let block_id = BlockId {
                        hash: header.block_hash(),
                        height: header.height,
                    };
                    self.cp = self.cp.clone().insert(block_id);
                }
            }
            _ => (),
        }
    }

    fn apply_block_event(&mut self, block: &IndexedBlock) {
        let height = block.height;
        let block = &block.block;
        let _ = self.graph.apply_block_relevant(block, height);
    }

    #[inline]
    fn peek_scripts_from_scantype(&self, scan_type: ScanType) -> HashSet<ScriptBuf> {
        match scan_type {
            ScanType::Sync => self.peek_script_to_keychain_lookahead(),
            ScanType::Recovery {
                used_script_index,
                checkpoint: _,
            } => self.peek_scripts(used_script_index),
        }
    }

    #[inline]
    fn peek_script_to_keychain_lookahead(&self) -> HashSet<ScriptBuf> {
        self.peek_scripts(self.graph.index.lookahead())
    }

    fn peek_scripts(&self, to_index: u32) -> HashSet<ScriptBuf> {
        let mut spk_cache = HashSet::new();
        let keychain = &self.graph.index;
        // We pre-compute an SPK cache so as to not call `unbounded_spk_iter` for each filter
        let last_revealed = keychain.last_revealed_indices();
        let ext_index = last_revealed
            .get(&KeychainKind::External)
            .copied()
            .unwrap_or(0);
        let unbounded_ext_spk_iter = keychain
            .unbounded_spk_iter(KeychainKind::External)
            .expect("wallet must have external keychain");
        let bound = (ext_index + to_index) as usize;
        let bounded_ext_iter = unbounded_ext_spk_iter.take(bound).map(|(_, script)| script);
        spk_cache.extend(bounded_ext_iter);
        let int_index = last_revealed
            .get(&KeychainKind::Internal)
            .copied()
            .unwrap_or(0);
        let unbounded_int_spk_iter = keychain.unbounded_spk_iter(KeychainKind::Internal);
        if let Some(int_spk_iter) = unbounded_int_spk_iter {
            let bound = (int_index + to_index) as usize;
            let bounded_int_iter = int_spk_iter.take(bound).map(|(_, script)| script);
            spk_cache.extend(bounded_int_iter);
        }
        spk_cache
    }

    fn finish(&mut self) -> Update {
        let tx_update = TxUpdate::from(self.graph.graph().clone());
        let graph = core::mem::take(&mut self.graph);
        let last_active_indices = graph.index.last_used_indices();
        self.graph = IndexedTxGraph::new(graph.index);
        Update {
            tx_update,
            last_active_indices,
            chain: Some(self.cp.clone()),
        }
    }
}

/// Errors encountered when attempting to construct a wallet update.
#[derive(Debug, Clone, Copy)]
pub enum UpdateError {
    /// The node has stopped running.
    NodeStopped,
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::NodeStopped => write!(f, "the node halted execution."),
        }
    }
}

impl std::error::Error for UpdateError {}

/// How to scan compact block filters on start up.
#[derive(Debug, Clone, Copy, Default)]
pub enum ScanType {
    /// Sync the wallet from the last known wallet checkpoint to the rest of the network.
    #[default]
    Sync,
    /// Recover an old wallet by scanning after the specified height.
    Recovery {
        /// The amount of scripts used by the wallet that is being recovered.
        used_script_index: u32,
        /// The height in the block chain to begin searching for transactions.
        checkpoint: HeaderCheckpoint,
    },
}
