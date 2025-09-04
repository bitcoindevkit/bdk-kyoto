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
//! use bdk_kyoto::kyoto::{Network, TrustedPeer};
//! use bdk_kyoto::builder::{NodeBuilder, NodeBuilderExt};
//! use bdk_kyoto::{LightClient, ScanType};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Add specific peers to connect to.
//!     let peer = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
//!     let trusted = TrustedPeer::from_ip(peer);
//!
//!     let db_path = ".".parse::<PathBuf>()?;
//!
//!     let mut wallet = Wallet::create(RECEIVE, CHANGE)
//!         .network(Network::Signet)
//!         .create_wallet_no_persist()?;
//!
//!     let scan_type = ScanType::Recovery { from_height: 200_000 };
//!
//!     let LightClient {
//!         requester,
//!         info_subscriber,
//!         warning_subscriber,
//!         update_subscriber,
//!         node
//!     } = NodeBuilder::new(Network::Signet)
//!         // A node may handle multiple connections
//!         .required_peers(2)
//!         // Choose where to store node data
//!         .data_dir(db_path)
//!         // How long peers have to respond messages
//!         .response_timeout(Duration::from_secs(2))
//!         // Added trusted peers to initialize the sync
//!         .add_peer(trusted)
//!         .build_with_wallet(&wallet, scan_type)?;
//!     Ok(())
//! }
//! ```

use std::fmt::Display;

use bdk_wallet::{chain::IndexedTxGraph, Wallet};
use kyoto::HeaderCheckpoint;
pub use kyoto::{db::error::SqlInitializationError, NodeBuilder};

use crate::{LightClient, ScanType, UpdateSubscriber, WalletExt};

/// Build a compact block filter client and node for a specified wallet
pub trait NodeBuilderExt {
    /// Attempt to build the node with scripts from a [`Wallet`] and following a [`ScanType`].
    fn build_with_wallet(
        self,
        wallet: &Wallet,
        scan_type: ScanType,
    ) -> Result<LightClient, BuilderError>;
}

impl NodeBuilderExt for NodeBuilder {
    fn build_with_wallet(
        mut self,
        wallet: &Wallet,
        scan_type: ScanType,
    ) -> Result<LightClient, BuilderError> {
        let network = wallet.network();
        if self.network().ne(&network) {
            return Err(BuilderError::NetworkMismatch);
        }
        let scripts = wallet.peek_revealed_plus_lookahead();
        self = self.add_scripts(scripts);
        match scan_type {
            // This is a no-op because kyoto will start from the latest checkpoint if none is
            // provided
            ScanType::New => (),
            ScanType::Sync => {
                let block_id = wallet.latest_checkpoint();
                let header_cp = HeaderCheckpoint::new(block_id.height(), block_id.hash());
                self = self.after_checkpoint(header_cp);
            }
            ScanType::Recovery { from_height } => {
                // Make sure we don't miss the first transaction of the wallet.
                // The anchor checkpoint is non-inclusive.
                let birthday = from_height.saturating_sub(1);
                let header_cp =
                    HeaderCheckpoint::closest_checkpoint_below_height(birthday, network);
                self = self.after_checkpoint(header_cp);
            }
        };
        let (node, client) = self.build()?;
        let kyoto::Client {
            requester,
            info_rx,
            warn_rx,
            event_rx,
        } = client;
        let indexed_graph = IndexedTxGraph::new(wallet.spk_index().clone());
        let update_subscriber = UpdateSubscriber {
            receiver: event_rx,
            cp: wallet.latest_checkpoint(),
            graph: indexed_graph,
        };
        Ok(LightClient {
            requester,
            info_subscriber: info_rx,
            warning_subscriber: warn_rx,
            update_subscriber,
            node,
        })
    }
}

/// Errors the builder may throw.
#[derive(Debug)]
pub enum BuilderError {
    /// The database failed to open.
    IO(SqlInitializationError),
    /// The wallet network and node network do not match.
    NetworkMismatch,
}

impl Display for BuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuilderError::IO(e) => write!(f, "failed to initialize the db: {e}"),
            BuilderError::NetworkMismatch => {
                write!(f, "wallet network and node network do not match")
            }
        }
    }
}

impl std::error::Error for BuilderError {}

impl From<SqlInitializationError> for BuilderError {
    fn from(value: SqlInitializationError) -> Self {
        BuilderError::IO(value)
    }
}
