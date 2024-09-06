//! Handle messages from a node.
//!
//! # Examples
//!
//! For quick iteration and debugging, the [`PrintLogger`] responds to node events by simply printing the display to the console.
//!
//! ```rust
//! use bdk_kyoto::logger::{PrintLogger, NodeMessageHandler};
//! use bdk_kyoto::Warning;
//!     
//! let logger = PrintLogger::new();
//! logger.dialog("The node is running".into());
//! logger.warning(Warning::PeerTimedOut);
//! ```
//!
//! For a more descriptive console log, the `tracing` feature may be used.
//!
//! ```rust
//! use bdk_kyoto::logger::{TraceLogger, NodeMessageHandler};
//! use bdk_kyoto::Warning;
//!     
//! let logger = TraceLogger::new();
//! logger.dialog("The node is running".into());
//! logger.warning(Warning::PeerTimedOut);
//! ```
//!
//! For production applications, a custom implementation of [`NodeMessageHandler`] should be implemented.
//! A good applciation logger should implement user interface behavior and potentially save information to a file.

use std::fmt::Debug;

use kyoto::NodeState;
use kyoto::Txid;
use kyoto::Warning;

/// Handle dialog and state changes from a node with some arbitrary behavior.
/// The primary purpose of this trait is not to respond to events by persisting changes,
/// or acting on the underlying wallet. Instead, this trait should be used to drive changes in user
/// interface behavior or keep a simple log. Relevant events that effect on the wallet are handled
/// automatically in [`Client::update`](crate::Client).
pub trait NodeMessageHandler: Send + Sync + Debug + 'static {
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
    fn tx_failed(&self, txid: Txid);
    /// A list of block heights were reorganized
    fn blocks_disconnected(&self, blocks: Vec<u32>);
    /// The node has synced to the height of the connected peers.
    fn synced(&self, tip: u32);
}

/// Print messages from the node to the console
#[derive(Default, Debug)]
pub struct PrintLogger {}

impl PrintLogger {
    /// Build a new print logger
    pub fn new() -> Self {
        Self {}
    }
}

impl NodeMessageHandler for PrintLogger {
    fn dialog(&self, dialog: String) {
        println!("{dialog}");
    }

    fn warning(&self, warning: Warning) {
        println!("{warning}");
    }

    fn state_changed(&self, state: NodeState) {
        println!("State change: {state}");
    }

    fn tx_sent(&self, txid: Txid) {
        println!("Transaction sent: {txid}");
    }

    fn tx_failed(&self, txid: Txid) {
        println!("Transaction failed: {txid}");
    }

    fn blocks_disconnected(&self, blocks: Vec<u32>) {
        for block in blocks {
            println!("Block {block} was reorganized");
        }
    }

    fn synced(&self, tip: u32) {
        println!("Synced to tip {tip}");
    }

    fn connections_met(&self) {
        println!("Required connections met");
    }
}

/// Print messages from the node to the console
#[cfg(feature = "trace")]
#[derive(Default, Debug)]
pub struct TraceLogger {}

#[cfg(feature = "trace")]
impl TraceLogger {
    /// Build a new trace logger
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(feature = "trace")]
impl NodeMessageHandler for TraceLogger {
    fn dialog(&self, dialog: String) {
        tracing::info!("{dialog}")
    }

    fn warning(&self, warning: Warning) {
        tracing::warn!("{warning}")
    }

    fn state_changed(&self, state: NodeState) {
        tracing::info!("State change: {state}")
    }

    fn tx_sent(&self, txid: Txid) {
        tracing::info!("Transaction sent: {txid}")
    }

    fn tx_failed(&self, txid: Txid) {
        tracing::info!("Transaction failed: {txid}")
    }

    fn blocks_disconnected(&self, blocks: Vec<u32>) {
        for block in blocks {
            tracing::warn!("Block {block} was reorganized");
        }
    }

    fn synced(&self, tip: u32) {
        tracing::info!("Synced to height: {tip}")
    }

    fn connections_met(&self) {
        tracing::info!("Required connections met")
    }
}

impl NodeMessageHandler for () {
    fn dialog(&self, _dialog: String) {}
    fn warning(&self, _warning: Warning) {}
    fn state_changed(&self, _state: NodeState) {}
    fn connections_met(&self) {}
    fn tx_sent(&self, _txid: Txid) {}
    fn tx_failed(&self, _txid: Txid) {}
    fn blocks_disconnected(&self, _blocks: Vec<u32>) {}
    fn synced(&self, _tip: u32) {}
}
