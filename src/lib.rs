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
//! use bdk_kyoto::LightClient;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut wallet = Wallet::create(RECEIVE, CHANGE)
//!         .network(Network::Signet)
//!         .create_wallet_no_persist()?;
//!
//!     let LightClient {
//!         requester,
//!         log_subscriber: _,
//!         warning_subscriber: _,
//!         mut update_subscriber,
//!         node
//!     } = LightClientBuilder::new().build(&wallet)?;
//!
//!     tokio::task::spawn(async move { node.run().await });
//!
//!     loop {
//!         if let Some(update) = update_subscriber.update().await {
//!             wallet.apply_update(update)?;
//!             return Ok(());
//!         }
//!     }
//! }
//! ```

#![warn(missing_docs)]
use core::{future::Future, pin::Pin};
use std::collections::BTreeMap;
use std::collections::HashSet;

type FutureResult<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

pub use bdk_wallet::chain::local_chain::MissingGenesisError;

use bdk_wallet::chain::{
    keychain_txout::KeychainTxOutIndex,
    local_chain::{self, CheckPoint, LocalChain},
    spk_client::FullScanResponse,
    IndexedTxGraph,
};
use bdk_wallet::chain::{ConfirmationBlockTime, TxUpdate};
use bdk_wallet::KeychainKind;

pub extern crate kyoto;

pub use kyoto::core::builder::NodeDefault;
pub use kyoto::FeeRate;
pub use kyoto::Requester;
pub use kyoto::{DisconnectedHeader, RejectPayload};
pub use kyoto::{
    Log, NodeState, ScriptBuf, SyncUpdate, TxBroadcast, TxBroadcastPolicy, Txid, Warning,
};

use kyoto::Receiver;
use kyoto::UnboundedReceiver;
use kyoto::{Event, IndexedBlock};

pub mod builder;

#[derive(Debug)]
/// A node and associated structs to send and receive events to and from the node.
pub struct LightClient {
    /// Send events to a running node (i.e. broadcast a transaction).
    pub requester: Requester,
    /// Receive logs from the node as it runs.
    pub log_subscriber: LogSubscriber,
    /// Receive warnings from the node as it runs.
    pub warning_subscriber: WarningSubscriber,
    /// Receive wallet updates from a node.
    pub update_subscriber: UpdateSubscriber,
    /// The underlying node that must be run to fetch blocks from peers.
    pub node: NodeDefault,
}

/// Interpret events from a node that is running to apply
/// updates to an underlying wallet.
#[derive(Debug)]
pub struct UpdateSubscriber {
    // channel receiver
    receiver: UnboundedReceiver<Event>,
    // changes to local chain
    chain: local_chain::LocalChain,
    // receive graph
    graph: IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<KeychainKind>>,
}

impl UpdateSubscriber {
    /// Build a light client event handler from a [`KeychainTxOutIndex`] and [`CheckPoint`].
    pub(crate) fn from_index(
        cp: CheckPoint,
        index: KeychainTxOutIndex<KeychainKind>,
        receiver: UnboundedReceiver<Event>,
    ) -> Result<Self, MissingGenesisError> {
        Ok(Self {
            receiver,
            chain: LocalChain::from_tip(cp)?,
            graph: IndexedTxGraph::new(index.clone()),
        })
    }

    /// Return the most recent update from the node once it has synced to the network's tip.
    /// This may take a significant portion of time during wallet recoveries or dormant wallets.
    /// Note that you may call this method in a loop as long as the node is running.
    ///
    /// A reference to a [`NodeEventHandler`] is required, which handles events emitted from a
    /// running node. Production applications should define how the application handles
    /// these events and displays them to end users.
    pub async fn update(&mut self) -> Option<FullScanResponse<KeychainKind>> {
        let mut chain_changeset = BTreeMap::new();
        while let Some(message) = self.receiver.recv().await {
            match message {
                Event::Block(IndexedBlock { height, block }) => {
                    let hash = block.header.block_hash();
                    chain_changeset.insert(height, Some(hash));
                    let _ = self.graph.apply_block_relevant(&block, height);
                }
                Event::BlocksDisconnected(headers) => {
                    for header in headers {
                        let height = header.height;
                        chain_changeset.insert(height, None);
                    }
                }
                Event::Synced(SyncUpdate {
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
            }
        }
        self.chain
            .apply_changeset(&local_chain::ChangeSet::from(chain_changeset))
            .expect("chain was initialized with genesis");
        Some(self.get_scan_response())
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
}

/// Receive logs from the node as it runs to drive user interface changes.
#[derive(Debug)]
pub struct LogSubscriber {
    receiver: Receiver<Log>,
}

impl LogSubscriber {
    pub(crate) fn new(receiver: Receiver<Log>) -> Self {
        Self { receiver }
    }

    /// Wait until the node emits a log for an indeterminant amount of time.
    pub async fn next_log(&mut self) -> Log {
        loop {
            if let Some(log) = self.receiver.recv().await {
                return log;
            }
        }
    }
}

/// Receive wanrings from the node to act on or to drive user interface changes
#[derive(Debug)]
pub struct WarningSubscriber {
    receiver: UnboundedReceiver<Warning>,
}

impl WarningSubscriber {
    pub(crate) fn new(receiver: UnboundedReceiver<Warning>) -> Self {
        Self { receiver }
    }

    /// Wait until the node emits a warning for an indeterminant amount of time.
    pub async fn next_warning(&mut self) -> Warning {
        loop {
            if let Some(warning) = self.receiver.recv().await {
                return warning;
            }
        }
    }
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

/// Extend the [`Requester`] functionality to work conveniently with a [`Wallet`](bdk_wallet).
pub trait RequesterExt {
    /// Add all revealed scripts to the node to monitor.
    fn add_revealed_scripts<'a>(
        &'a self,
        wallet: &'a bdk_wallet::Wallet,
    ) -> FutureResult<'a, (), kyoto::ClientError>;
}

impl RequesterExt for Requester {
    fn add_revealed_scripts<'a>(
        &'a self,
        wallet: &'a bdk_wallet::Wallet,
    ) -> FutureResult<'a, (), kyoto::ClientError> {
        async fn _add_revealed(
            requester: &Requester,
            wallet: &bdk_wallet::Wallet,
        ) -> Result<(), kyoto::ClientError> {
            for keychain in [KeychainKind::External, KeychainKind::Internal] {
                let scripts = wallet.spk_index().revealed_keychain_spks(keychain);
                for (_, script) in scripts {
                    requester.add_script(script).await?;
                }
            }
            Ok(())
        }
        Box::pin(_add_revealed(self, wallet))
    }
}
