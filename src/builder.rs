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
//! use std::path::PathBuf;
//! use std::time::Duration;
//! use bdk_wallet::Wallet;
//! use bdk_wallet::bitcoin::Network;
//! use bdk_kyoto::builder::{LightClientBuilder, TrustedPeer};
//! use bdk_kyoto::logger::PrintLogger;
//! use bdk_kyoto::LightClient;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Add specific peers to connect to.
//!     let peer = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
//!     let trusted = TrustedPeer::from_ip(peer);
//!     let peers = vec![trusted];
//!
//!     let db_path = ".".parse::<PathBuf>()?;
//!
//!     let mut wallet = Wallet::create(RECEIVE, CHANGE)
//!         .network(Network::Signet)
//!         .create_wallet_no_persist()?;
//!
//!     let LightClient { sender, receiver, node } = LightClientBuilder::new(&wallet)
//!         // When recovering a user's wallet, specify a height to start at
//!         .scan_after(200_000)
//!         // A node may handle mutliple connections
//!         .connections(2)
//!         // Choose where to store node data
//!         .data_dir(db_path)
//!         // How long peers have to respond messages
//!         .timeout_duration(Duration::from_secs(10))
//!         .peers(peers)
//!         .build()?;
//!     Ok(())
//! }
//! ```

use std::{collections::HashSet, path::PathBuf, time::Duration};

use bdk_chain::local_chain::MissingGenesisError;
use bdk_wallet::{KeychainKind, Wallet};
use kyoto::NodeBuilder;
pub use kyoto::{
    db::error::SqlInitializationError, AddrV2, HeaderCheckpoint, ScriptBuf, ServiceFlags,
    TrustedPeer,
};

use crate::{EventReceiver, LightClient};

const RECOMMENDED_PEERS: u8 = 2;

#[derive(Debug)]
/// Construct a light client from a [`Wallet`] reference.
pub struct LightClientBuilder<'a> {
    wallet: &'a Wallet,
    peers: Option<Vec<TrustedPeer>>,
    connections: Option<u8>,
    birthday_height: Option<u32>,
    data_dir: Option<PathBuf>,
    timeout: Option<Duration>,
}

impl<'a> LightClientBuilder<'a> {
    /// Construct a new node builder.
    pub fn new(wallet: &'a Wallet) -> Self {
        Self {
            wallet,
            peers: None,
            connections: None,
            birthday_height: None,
            data_dir: None,
            timeout: None,
        }
    }
    /// Add peers to connect to over the P2P network.
    pub fn peers(mut self, peers: Vec<TrustedPeer>) -> Self {
        self.peers = Some(peers);
        self
    }

    /// Add the number of connections for the node to maintain.
    pub fn connections(mut self, num_connections: u8) -> Self {
        self.connections = Some(num_connections);
        self
    }

    /// Add a directory to store node data
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }

    /// Add a wallet "birthday", or block to start searching for transactions _strictly after_.
    /// Only useful for recovering wallets. If the wallet has a tip that is already higher than the
    /// height provided, this height will be ignored.
    pub fn scan_after(mut self, height: u32) -> Self {
        self.birthday_height = Some(height);
        self
    }

    /// Configure the duration of time a remote node has to respond to a message.
    pub fn timeout_duration(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Build a light client node and a client to interact with the node.
    pub fn build(self) -> Result<LightClient, BuilderError> {
        let network = self.wallet.network();
        let mut node_builder = NodeBuilder::new(network);
        if let Some(whitelist) = self.peers {
            node_builder = node_builder.add_peers(whitelist);
        }
        match self.birthday_height {
            Some(birthday) => {
                // If there is a birthday at a height less than our local chain, we may assume we've
                // already synced the wallet past the birthday height and no longer
                // need it.
                if birthday < self.wallet.local_chain().tip().height() {
                    let block_id = self.wallet.local_chain().tip();
                    let header_cp = HeaderCheckpoint::new(block_id.height(), block_id.hash());
                    node_builder = node_builder.anchor_checkpoint(header_cp)
                } else {
                    let cp = HeaderCheckpoint::closest_checkpoint_below_height(birthday, network);
                    node_builder = node_builder.anchor_checkpoint(cp)
                }
            }
            None => {
                // If there is no birthday provided and the local chain starts at the genesis block,
                // we assume this is a new wallet and use the most recent
                // checkpoint. Otherwise we sync from the last known tip in the
                // LocalChain.
                let block_id = self.wallet.local_chain().tip();
                if block_id.height() > 0 {
                    let header_cp = HeaderCheckpoint::new(block_id.height(), block_id.hash());
                    node_builder = node_builder.anchor_checkpoint(header_cp)
                }
            }
        }
        if let Some(dir) = self.data_dir {
            node_builder = node_builder.add_data_dir(dir);
        }
        if let Some(duration) = self.timeout {
            node_builder = node_builder.set_response_timeout(duration)
        }
        node_builder =
            node_builder.num_required_peers(self.connections.unwrap_or(RECOMMENDED_PEERS));
        let mut spks: HashSet<ScriptBuf> = HashSet::new();
        for keychain in [KeychainKind::External, KeychainKind::Internal] {
            // The user may choose to recover a wallet with lookahead scripts
            // or use the last revealed index plus some padding to find new transactions
            let last_revealed = self
                .wallet
                .spk_index()
                .last_revealed_index(keychain)
                .unwrap_or(0);
            let lookahead_index = last_revealed + self.wallet.spk_index().lookahead();
            for index in 0..=lookahead_index {
                spks.insert(self.wallet.peek_address(keychain, index).script_pubkey());
            }
        }
        let (node, kyoto_client) = node_builder.add_scripts(spks).build_node()?;
        let (sender, receiver) = kyoto_client.split();
        let event_receiver = EventReceiver::from_index(
            self.wallet.local_chain().tip(),
            self.wallet.spk_index(),
            receiver,
        )?;
        Ok(LightClient {
            sender,
            receiver: event_receiver,
            node,
        })
    }
}

/// Errors thrown by the [`LightClientBuilder`].
#[derive(Debug)]
pub enum BuilderError {
    /// The `LocalChain` was not initialized with a genesis block.
    Chain(MissingGenesisError),
    /// The database encountered a fatal error.
    Database(SqlInitializationError),
}

impl std::fmt::Display for BuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuilderError::Chain(e) => write!(f, "genesis block not found: {e}"),
            BuilderError::Database(e) => write!(f, "fatal database error: {e}"),
        }
    }
}

impl std::error::Error for BuilderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BuilderError::Chain(e) => Some(e),
            BuilderError::Database(e) => Some(e),
        }
    }
}

impl From<MissingGenesisError> for BuilderError {
    fn from(value: MissingGenesisError) -> Self {
        BuilderError::Chain(value)
    }
}

impl From<SqlInitializationError> for BuilderError {
    fn from(value: SqlInitializationError) -> Self {
        BuilderError::Database(value)
    }
}
