//! Construct a [`Node`] and [`Client`] by using a reference to a [`Wallet`].
//! 
//! ## Details
//! 
//! ```no_run
//! # const RECEIVE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
//! # const CHANGE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
//! use std::net::{IpAddr, Ipv4Addr};
//! use std::path::PathBuf;
//! use std::time::Duration;
//! use bdk_wallet::Wallet;
//! use bdk_wallet::bitcoin::Network;
//! use bdk_kyoto::TrustedPeer;
//! use bdk_kyoto::builder::LightClientBuilder;
//! use bdk_kyoto::logger::PrintLogger;
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
//!     let (mut node, mut client) = LightClientBuilder::new(&wallet)
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

use std::{collections::HashSet, path::PathBuf, str::FromStr, time::Duration};

use bdk_wallet::{KeychainKind, Wallet};
use kyoto::{
    chain::checkpoints::{
        HeaderCheckpoint, MAINNET_HEADER_CP, REGTEST_HEADER_CP, SIGNET_HEADER_CP,
    },
    BlockHash, DatabaseError, Network, Node, NodeBuilder, ScriptBuf, TrustedPeer,
};

use crate::Client;

const TARGET_INDEX: u32 = 20;
const RECOMMENDED_PEERS: u8 = 2;

#[derive(Debug)]
/// Construct a light client from higher level components.
pub struct LightClientBuilder<'a> {
    wallet: &'a Wallet,
    peers: Option<Vec<TrustedPeer>>,
    connections: Option<u8>,
    birthday_height: Option<u32>,
    data_dir: Option<PathBuf>,
    timeout: Option<Duration>,
}

impl<'a> LightClientBuilder<'a> {
    /// Construct a new node builder
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
    pub fn data_dir(mut self, dir: PathBuf) -> Self {
        self.data_dir = Some(dir);
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

    // Get the most recent checkpoint that is less than the recovery height.
    fn get_checkpoint_for_height(height: u32, network: &Network) -> HeaderCheckpoint {
        let checkpoints: Vec<HeaderCheckpoint> = match network {
            Network::Bitcoin => MAINNET_HEADER_CP
                .iter()
                .copied()
                .map(|(height, hash)| {
                    HeaderCheckpoint::new(height, BlockHash::from_str(hash).unwrap())
                })
                .collect(),
            Network::Testnet => panic!(),
            Network::Signet => SIGNET_HEADER_CP
                .iter()
                .copied()
                .map(|(height, hash)| {
                    HeaderCheckpoint::new(height, BlockHash::from_str(hash).unwrap())
                })
                .collect(),
            Network::Regtest => REGTEST_HEADER_CP
                .iter()
                .copied()
                .map(|(height, hash)| {
                    HeaderCheckpoint::new(height, BlockHash::from_str(hash).unwrap())
                })
                .collect(),
            _ => unreachable!(),
        };
        let mut cp = *checkpoints.first().unwrap();
        for checkpoint in checkpoints {
            if height.ge(&checkpoint.height) {
                cp = checkpoint;
            } else {
                break;
            }
        }
        cp
    }

    /// Build a light client node and a client to interact with the node
    pub fn build(self) -> Result<(Node, Client<KeychainKind>), Error> {
        let network = self.wallet.network();
        let mut node_builder = NodeBuilder::new(network);
        if let Some(whitelist) = self.peers {
            node_builder = node_builder.add_peers(whitelist);
        }
        match self.birthday_height {
            Some(birthday) => {
                // If there is a birthday at a height less than our local chain, we may assume we've already synced
                // the wallet past the birthday height and no longer need it.
                if birthday < self.wallet.local_chain().tip().height() {
                    let block_id = self.wallet.local_chain().tip();
                    let header_cp = HeaderCheckpoint::new(block_id.height(), block_id.hash());
                    node_builder = node_builder.anchor_checkpoint(header_cp)
                } else {
                    let cp = Self::get_checkpoint_for_height(birthday, &network);
                    node_builder = node_builder.anchor_checkpoint(cp)
                }
            }
            None => {
                // If there is no birthday provided and the local chain starts at the genesis block, we assume this
                // is a new wallet and use the most recent checkpoint. Otherwise we sync from the last known tip in the
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
        // Reveal 20 scripts ahead of the last revealed index so we don't miss any transactions.
        for keychain in [KeychainKind::External, KeychainKind::Internal] {
            for index in 0..=self
                .wallet
                .spk_index()
                .last_revealed_index(keychain)
                .unwrap_or(0)
                + TARGET_INDEX
            {
                spks.insert(self.wallet.peek_address(keychain, index).script_pubkey());
            }
        }
        let (node, kyoto_client) = node_builder.add_scripts(spks).build_node()?;
        let client = Client::from_index(
            self.wallet.local_chain().tip(),
            self.wallet.spk_index(),
            kyoto_client,
        )?;
        Ok((node, client))
    }
}

/// Errors thrown by a client.
#[derive(Debug)]
pub enum Error {
    /// The `LocalChain` was not initialized with a genesis block.
    MissingGenesis(crate::Error),
    /// The database encountered a fatal error.
    Database(DatabaseError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::MissingGenesis(e) => write!(f, "genesis block not found: {e}"),
            Error::Database(e) => write!(f, "fatal database error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::MissingGenesis(e) => Some(e),
            Error::Database(e) => Some(e),
        }
    }
}

impl From<crate::Error> for Error {
    fn from(value: crate::Error) -> Self {
        Error::MissingGenesis(value)
    }
}

impl From<DatabaseError> for Error {
    fn from(value: DatabaseError) -> Self {
        Error::Database(value)
    }
}
