//! Construct a [`LightClient`] by using a reference to a [`Wallet`].
//!
//! ## Details
//!
//! The node has a number of configurations. Notably, the height of in the blockchain to start a
//! wallet recovery and the nodes on the peer-to-peer network are both configurable.
//!
//! ```no_run
//! # const RECEIVE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
//! # const CHANGE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
//! use std::net::{IpAddr, Ipv4Addr};
//! use std::time::Duration;
//! use bdk_wallet::Wallet;
//! use bdk_kyoto::bip157::{Network, TrustedPeer};
//! use bdk_kyoto::builder::{Builder, BuilderExt};
//! use bdk_kyoto::{LightClient, SyncConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Add specific peers to connect to.
//!     let peer = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
//!     let trusted = TrustedPeer::from_ip(peer);
//!
//!     let mut wallet = Wallet::create(RECEIVE, CHANGE)
//!         .network(Network::Signet)
//!         .create_wallet_no_persist()?;
//!
//!     let sync_config = SyncConfig::sync_from_last_checkpoint().build();
//!
//!     let client = Builder::new(Network::Signet)
//!         // A node may handle multiple connections
//!         .required_peers(2)
//!         // How long peers have to respond messages
//!         .response_timeout(Duration::from_secs(2))
//!         // Added trusted peers to initialize the sync
//!         .add_peer(trusted)
//!         .build_with_wallet(&wallet, sync_config)?;
//!     Ok(())
//! }
//! ```
//!
//! Multiple wallets are synced by one node with [`BuilderExt::wallets`], each following its own
//! [`SyncConfig`].
//!
//! ```no_run
//! # const RECEIVE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
//! # const CHANGE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
//! # const RECEIVE_TWO: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/0/*)";
//! # const CHANGE_TWO: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/1/*)";
//! use bdk_wallet::Wallet;
//! use bdk_kyoto::bip157::Network;
//! use bdk_kyoto::builder::{Builder, BuilderExt};
//! use bdk_kyoto::SyncConfig;
//!
//! # fn main() -> anyhow::Result<()> {
//! let existing = Wallet::create(RECEIVE, CHANGE)
//!     .network(Network::Signet)
//!     .create_wallet_no_persist()?;
//! let new = Wallet::create(RECEIVE_TWO, CHANGE_TWO)
//!     .network(Network::Signet)
//!     .create_wallet_no_persist()?;
//!
//! let client = Builder::new(Network::Signet)
//!     .wallets()
//!     .add(&existing, SyncConfig::sync_from_last_checkpoint().build())
//!     .add(&new, SyncConfig::new_wallet_sync().build())
//!     .build()?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashSet;
use std::fmt::Display;

use bdk_wallet::{
    chain::{CheckPoint, DescriptorExt, DescriptorId, IndexedTxGraph},
    KeychainKind, Wallet,
};
pub use bip157::Builder;
use bip157::{chain::ChainState, HashCheckpoint, Network};

use crate::{
    state::Idle, LightClient, LoggingSubscribers, SyncConfig, SyncPolicy, SyncPolicyType,
    UpdateSubscriber,
};

const IMPOSSIBLE_REORG_DEPTH: usize = 7;

/// Build a compact block filter client and node for a specified wallet
pub trait BuilderExt {
    /// Attempt to build the node with scripts from a [`Wallet`] and following a [`SyncConfig`].
    fn build_with_wallet<P: SyncPolicyType>(
        self,
        wallet: &Wallet,
        sync_config: SyncConfig<P>,
    ) -> Result<LightClient<Idle, crate::wallets::Single>, BuilderError>;

    /// Sync multiple [`Wallet`]s with one node. Each wallet is added with its own
    /// [`SyncConfig`] by [`WalletsBuilder::add`].
    fn wallets<'a>(self) -> WalletsBuilder<'a>;
}

impl BuilderExt for Builder {
    fn build_with_wallet<P: SyncPolicyType>(
        mut self,
        wallet: &Wallet,
        sync_config: SyncConfig<P>,
    ) -> Result<LightClient<Idle, crate::wallets::Single>, BuilderError> {
        let network = wallet.network();
        if self.network().ne(&network) {
            return Err(BuilderError::NetworkMismatch);
        }
        match sync_config.0 {
            SyncPolicy::NewWallet => {
                self = self
                    .chain_state(ChainState::Checkpoint(new_wallet_anchor(network)))
                    .headers_only_sync();
            }
            SyncPolicy::SyncFromLast => {
                let sync_start = walk_back_max_reorg(wallet.latest_checkpoint());
                self = self.chain_state(ChainState::Checkpoint(sync_start));
            }
            SyncPolicy::Recovery { cp } => {
                self = self.chain_state(ChainState::Checkpoint(cp));
            }
        }
        let (node, client) = self.build();
        let bip157::Client {
            requester,
            info_rx,
            warn_rx,
            event_rx,
        } = client;
        let indexed_graph = IndexedTxGraph::new(wallet.spk_index().clone());
        let update_subscriber = UpdateSubscriber::<crate::wallets::Single>::new(
            requester.clone(),
            event_rx,
            wallet.latest_checkpoint(),
            indexed_graph,
        );
        let client = LightClient::new(
            requester,
            LoggingSubscribers {
                info_subscriber: info_rx,
                warning_subscriber: warn_rx,
            },
            update_subscriber,
            node,
        );
        Ok(client)
    }

    fn wallets<'a>(self) -> WalletsBuilder<'a> {
        WalletsBuilder {
            builder: self,
            wallets: Vec::new(),
        }
    }
}

/// Build a [`LightClient`] that syncs multiple [`Wallet`]s, each following its own
/// [`SyncConfig`].
///
/// The node starts from the earliest point required by the wallets synced from their last
/// checkpoint or recovered. New wallets do not move the starting point, but blocks paying their
/// scripts after it are still found. If every wallet is new, only block headers are synced up to
/// the current tip, as for a single new wallet.
pub struct WalletsBuilder<'a> {
    builder: Builder,
    wallets: Vec<(&'a Wallet, SyncPolicy)>,
}

impl<'a> WalletsBuilder<'a> {
    /// Add a [`Wallet`] to sync, following a [`SyncConfig`].
    pub fn add<P: SyncPolicyType>(
        mut self,
        wallet: &'a Wallet,
        sync_config: SyncConfig<P>,
    ) -> Self {
        self.wallets.push((wallet, sync_config.0));
        self
    }

    /// Attempt to build the node with scripts from every added [`Wallet`].
    pub fn build(self) -> Result<LightClient<Idle, crate::wallets::Multiple>, BuilderError> {
        let WalletsBuilder {
            mut builder,
            wallets,
        } = self;
        if wallets.is_empty() {
            return Err(BuilderError::EmptyIterator);
        }
        let network = builder.network();
        let mut ids = HashSet::new();
        for (wallet, _) in &wallets {
            if wallet.network().ne(&network) {
                return Err(BuilderError::NetworkMismatch);
            }
            if !ids.insert(external_descriptor_id(wallet)) {
                return Err(BuilderError::DuplicateWallet);
            }
        }
        let sync_start = wallets
            .iter()
            .filter_map(|(wallet, policy)| match policy {
                SyncPolicy::NewWallet => None,
                SyncPolicy::SyncFromLast => Some(walk_back_max_reorg(wallet.latest_checkpoint())),
                SyncPolicy::Recovery { cp } => Some(*cp),
            })
            .min();
        builder = match sync_start {
            Some(cp) => builder.chain_state(ChainState::Checkpoint(cp)),
            None => builder
                .chain_state(ChainState::Checkpoint(new_wallet_anchor(network)))
                .headers_only_sync(),
        };
        let (node, client) = builder.build();
        let bip157::Client {
            requester,
            info_rx,
            warn_rx,
            event_rx,
        } = client;
        let wallet_iter = wallets.into_iter().map(|(wallet, _)| {
            (
                external_descriptor_id(wallet),
                wallet.latest_checkpoint(),
                IndexedTxGraph::new(wallet.spk_index().clone()),
            )
        });
        let update_subscriber = UpdateSubscriber::<crate::wallets::Multiple>::new_multiple(
            requester.clone(),
            event_rx,
            wallet_iter,
        );
        let client = LightClient::new(
            requester,
            LoggingSubscribers {
                info_subscriber: info_rx,
                warning_subscriber: warn_rx,
            },
            update_subscriber,
            node,
        );
        Ok(client)
    }
}

fn external_descriptor_id(wallet: &Wallet) -> DescriptorId {
    wallet
        .public_descriptor(KeychainKind::External)
        .descriptor_id()
}

fn new_wallet_anchor(network: Network) -> HashCheckpoint {
    match network {
        Network::Bitcoin => HashCheckpoint::taproot_activation(),
        net => HashCheckpoint::from_genesis(net),
    }
}

/// Walk back 7 blocks in case the last sync was an orphan block.
fn walk_back_max_reorg(checkpoint: CheckPoint) -> HashCheckpoint {
    let mut ret_cp = HashCheckpoint::new(checkpoint.height(), checkpoint.hash());
    let cp_iter = checkpoint.iter();
    for (index, next) in cp_iter.enumerate() {
        if index > IMPOSSIBLE_REORG_DEPTH {
            return ret_cp;
        }
        ret_cp = HashCheckpoint::new(next.height(), next.hash());
    }
    ret_cp
}

/// Errors the builder may throw.
#[derive(Debug)]
pub enum BuilderError {
    /// The wallet network and node network do not match.
    NetworkMismatch,
    /// No wallets were added.
    EmptyIterator,
    /// The same wallet was added more than once.
    DuplicateWallet,
}

impl Display for BuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuilderError::NetworkMismatch => {
                write!(f, "wallet network and node network do not match")
            }
            BuilderError::EmptyIterator => write!(f, "no wallets were added"),
            BuilderError::DuplicateWallet => write!(f, "the same wallet was added more than once"),
        }
    }
}

impl std::error::Error for BuilderError {}
