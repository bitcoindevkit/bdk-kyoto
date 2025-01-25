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

use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
    net::IpAddr,
    path::PathBuf,
    time::Duration,
};

use bdk_wallet::{chain::IndexedTxGraph, Wallet};
pub use kyoto::{
    db::error::SqlInitializationError, AddrV2, HeaderCheckpoint, LogLevel, ScriptBuf, ServiceFlags,
    TrustedPeer,
};
use kyoto::{Network, NodeBuilder};

use crate::{
    multi::{MultiSyncRequest, MultiUpdateSubscriber},
    LightClient, MultiLightClient, ScanType, UpdateSubscriber, WalletExt,
};

const RECOMMENDED_PEERS: u8 = 2;

#[derive(Debug)]
/// Construct a light client from a [`Wallet`] reference.
pub struct LightClientBuilder {
    peers: Option<Vec<TrustedPeer>>,
    connections: Option<u8>,
    scan_type: ScanType,
    data_dir: Option<PathBuf>,
    timeout: Option<Duration>,
    dns_resolver: Option<IpAddr>,
    log_level: LogLevel,
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
            dns_resolver: None,
            log_level: LogLevel::default(),
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

    /// Set the [`LogLevel`] of the node.
    pub fn log_level(mut self, log_level: LogLevel) -> Self {
        self.log_level = log_level;
        self
    }

    /// Configure the duration of time a remote node has to respond to a message.
    pub fn timeout_duration(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Configure a DNS resolver to use when querying DNS seeds.
    pub fn dns_resolver(mut self, ip_addr: impl Into<IpAddr>) -> Self {
        self.dns_resolver = Some(ip_addr.into());
        self
    }

    fn common_builder(&self, network: Network) -> NodeBuilder {
        let mut node_builder = NodeBuilder::new(network);
        if let Some(whitelist) = self.peers.clone() {
            node_builder = node_builder.add_peers(whitelist);
        }
        if let Some(dir) = &self.data_dir {
            node_builder = node_builder.data_dir(dir);
        }
        if let Some(duration) = self.timeout {
            node_builder = node_builder.response_timeout(duration);
        }
        if let Some(dns_resolver) = self.dns_resolver {
            node_builder = node_builder.dns_resolver(dns_resolver);
        }
        node_builder = node_builder.log_level(self.log_level);
        node_builder = node_builder.required_peers(self.connections.unwrap_or(RECOMMENDED_PEERS));
        node_builder
    }

    /// Build a light client node and a client to interact with the node.
    pub fn build(self, wallet: &Wallet) -> Result<LightClient, SqlInitializationError> {
        let network = wallet.network();
        let mut node_builder = self.common_builder(network);
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
        let (node, kyoto_client) = node_builder
            .add_scripts(wallet.peek_revealed_plus_lookahead())
            .build()?;
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
            log_subscriber: log_rx,
            warning_subscriber: warn_rx,
            update_subscriber,
            node,
        })
    }

    /// Built a client and node for multiple wallets.
    pub fn build_multi<'a, H: Hash + Eq + Clone + Copy>(
        self,
        wallet_requests: impl IntoIterator<Item = MultiSyncRequest<'a, H>>,
    ) -> Result<MultiLightClient<H>, MultiWalletBuilderError> {
        let wallet_requests: Vec<MultiSyncRequest<'a, H>> = wallet_requests.into_iter().collect();
        let network_ref = wallet_requests
            .first()
            .ok_or(MultiWalletBuilderError::EmptyWalletList)
            .map(|request| request.wallet.network())?;
        for network in wallet_requests.iter().map(|req| req.wallet.network()) {
            if network_ref.ne(&network) {
                return Err(MultiWalletBuilderError::NetworkMismatch);
            }
        }
        let mut node_builder = self.common_builder(network_ref);
        let mut checkpoints = Vec::new();

        for wallet_request in wallet_requests.iter() {
            match wallet_request.scan_type {
                ScanType::New => (),
                ScanType::Sync => {
                    let cp = wallet_request.wallet.latest_checkpoint();
                    checkpoints.push(HeaderCheckpoint::new(cp.height(), cp.hash()))
                }
                ScanType::Recovery { from_height } => {
                    checkpoints.push(HeaderCheckpoint::closest_checkpoint_below_height(
                        from_height,
                        network_ref,
                    ));
                }
            }
        }
        if let Some(min) = checkpoints.into_iter().min_by_key(|h| h.height) {
            node_builder = node_builder.anchor_checkpoint(min);
        }

        let mut wallet_map = HashMap::new();
        for wallet_request in wallet_requests {
            let chain = wallet_request.wallet.local_chain().clone();
            let keychain_index = wallet_request.wallet.spk_index().clone();
            let indexed_graph = IndexedTxGraph::new(keychain_index);
            wallet_map.insert(wallet_request.index, (chain, indexed_graph));
            node_builder =
                node_builder.add_scripts(wallet_request.wallet.peek_revealed_plus_lookahead());
        }

        let (node, kyoto_client) = node_builder.build()?;
        let kyoto::Client {
            requester,
            log_rx,
            warn_rx,
            event_rx,
        } = kyoto_client;
        let multi_update = MultiUpdateSubscriber {
            receiver: event_rx,
            wallet_map,
            chain_changeset: BTreeMap::new(),
        };
        let client = MultiLightClient {
            requester,
            log_subscriber: log_rx,
            warning_subscriber: warn_rx,
            update_subscriber: multi_update,
            node,
        };
        Ok(client)
    }
}

impl Default for LightClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that may occur when attempting to build a node for a list of wallets.
#[derive(Debug)]
pub enum MultiWalletBuilderError {
    /// Two or more wallets do not have the same network configured.
    NetworkMismatch,
    /// The database encountered an error when attempting to open a connection.
    SqlError(SqlInitializationError),
    /// The list of wallets provided was empty.
    EmptyWalletList,
}

impl std::fmt::Display for MultiWalletBuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self {
            MultiWalletBuilderError::SqlError(error) => write!(f, "{error}"),
            MultiWalletBuilderError::NetworkMismatch => write!(
                f,
                "two or more wallets do not have the same network configured."
            ),
            MultiWalletBuilderError::EmptyWalletList => {
                write!(f, "no wallets were present in the iterator.")
            }
        }
    }
}

impl std::error::Error for MultiWalletBuilderError {}

impl From<SqlInitializationError> for MultiWalletBuilderError {
    fn from(value: SqlInitializationError) -> Self {
        MultiWalletBuilderError::SqlError(value)
    }
}
