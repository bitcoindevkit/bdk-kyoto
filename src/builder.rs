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
//! use bdk_kyoto::{LightClient, ScanType};
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
//!     let scan_type = ScanType::Recovery { from_height: 200_000 };
//!
//!     let LightClient { requester, log_subscriber, warning_subscriber, update_subscriber, node } = LightClientBuilder::new()
//!         // Configure the scan to recover the wallet
//!         .scan_type(scan_type)
//!         // A node may handle mutliple connections
//!         .connections(2)
//!         // Choose where to store node data
//!         .data_dir(db_path)
//!         // How long peers have to respond messages
//!         .timeout_duration(Duration::from_secs(10))
//!         .peers(peers)
//!         .build(&wallet)?;
//!     Ok(())
//! }
//! ```

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use bdk_wallet::{chain::IndexedTxGraph, Wallet};
use kyoto::NodeBuilder;
pub use kyoto::{
    db::error::SqlInitializationError, AddrV2, HeaderCheckpoint, ScriptBuf, ServiceFlags,
    TrustedPeer,
};

use crate::{LightClient, LogSubscriber, ScanType, UpdateSubscriber, WalletExt, WarningSubscriber};

const RECOMMENDED_PEERS: u8 = 2;

#[derive(Debug)]
/// Construct a light client from a [`Wallet`] reference.
pub struct LightClientBuilder {
    peers: Option<Vec<TrustedPeer>>,
    connections: Option<u8>,
    scan_type: ScanType,
    data_dir: Option<PathBuf>,
    timeout: Option<Duration>,
}

impl LightClientBuilder {
    /// Construct a new node builder.
    pub fn new() -> Self {
        Self {
            peers: None,
            connections: None,
            scan_type: ScanType::default(),
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

    /// Add a wallet "birthday", or block to start searching for transactions.
    /// Implicitly sets the [`ScanType`] to `ScanType::Recovery`.
    pub fn scan_after(mut self, height: u32) -> Self {
        self.scan_type = ScanType::Recovery {
            from_height: height,
        };
        self
    }

    /// Set the [`ScanType`] to sync, recover or start a new wallet.
    pub fn scan_type(mut self, scan_type: ScanType) -> Self {
        self.scan_type = scan_type;
        self
    }

    /// Configure the duration of time a remote node has to respond to a message.
    pub fn timeout_duration(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Build a light client node and a client to interact with the node.
    pub fn build(self, wallet: &Wallet) -> Result<LightClient, SqlInitializationError> {
        let network = wallet.network();
        let mut node_builder = NodeBuilder::new(network);
        if let Some(whitelist) = self.peers {
            node_builder = node_builder.add_peers(whitelist);
        }
        match self.scan_type {
            // This is a no-op because kyoto will start from the latest checkpoint if none is
            // provided
            ScanType::New => (),
            ScanType::Sync => {
                let block_id = wallet.local_chain().tip();
                let header_cp = HeaderCheckpoint::new(block_id.height(), block_id.hash());
                node_builder = node_builder.anchor_checkpoint(header_cp);
            }
            ScanType::Recovery { from_height } => {
                // Make sure we don't miss the first transaction of the wallet.
                // The anchor checkpoint is non-inclusive.
                let birthday = from_height.saturating_sub(1);
                let header_cp =
                    HeaderCheckpoint::closest_checkpoint_below_height(birthday, network);
                node_builder = node_builder.anchor_checkpoint(header_cp);
            }
        };

        if let Some(dir) = self.data_dir {
            node_builder = node_builder.add_data_dir(dir);
        }
        if let Some(duration) = self.timeout {
            node_builder = node_builder.set_response_timeout(duration)
        }
        node_builder =
            node_builder.num_required_peers(self.connections.unwrap_or(RECOMMENDED_PEERS));
        let (node, kyoto_client) = node_builder
            .add_scripts(wallet.peek_revealed_plus_lookahead().collect())
            .build_node()?;
        let kyoto::Client {
            requester,
            log_rx,
            warn_rx,
            event_rx,
        } = kyoto_client;
        let indexed_graph = IndexedTxGraph::new(wallet.spk_index().clone());
        let update_subscriber = UpdateSubscriber {
            receiver: event_rx,
            chain: wallet.local_chain().clone(),
            graph: indexed_graph,
            chain_changeset: BTreeMap::new(),
        };
        Ok(LightClient {
            requester,
            log_subscriber: LogSubscriber::new(log_rx),
            warning_subscriber: WarningSubscriber::new(warn_rx),
            update_subscriber,
            node,
        })
    }
}

impl Default for LightClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}
