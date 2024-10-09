//! Handle messages from a node.
//!
//! # Examples
//!
//! For quick iteration and debugging, the [`PrintLogger`] responds to node events by simply
//! printing the display to the console.
//!
//! ```rust
//! use bdk_kyoto::logger::PrintLogger;
//! use bdk_kyoto::Warning;
//! use bdk_kyoto::NodeEventHandler;
//!
//! let logger = PrintLogger::new();
//! logger.dialog("The node is running".into());
//! logger.warning(Warning::PeerTimedOut);
//! ```
//!
//! For a more descriptive console log, the `tracing` feature may be used.
//!
//! ```rust
//! use bdk_kyoto::logger::TraceLogger;
//! use bdk_kyoto::Warning;
//! use bdk_kyoto::NodeEventHandler;
//!
//! let logger = TraceLogger::new().unwrap();
//! logger.dialog("The node is running".into());
//! logger.warning(Warning::PeerTimedOut);
//! ```
//!
//! For production applications, a custom implementation of [`NodeEventHandler`] should be
//! implemented. An example of a good applciation logger should implement user interface behavior
//! and potentially save information to a file.

use std::fmt::Debug;

use kyoto::NodeState;
use kyoto::Txid;
use kyoto::Warning;
#[cfg(feature = "trace")]
use tracing::subscriber::SetGlobalDefaultError;

use crate::NodeEventHandler;

/// Print messages from the node to the console
#[derive(Default, Debug)]
pub struct PrintLogger {}

impl PrintLogger {
    /// Build a new print logger
    pub fn new() -> Self {
        Self {}
    }
}

impl NodeEventHandler for PrintLogger {
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

/// Print messages from the node to the console using [`tracing`].
#[cfg(feature = "trace")]
#[derive(Default, Debug)]
pub struct TraceLogger {}

#[cfg(feature = "trace")]
impl TraceLogger {
    /// Build a new trace logger. This constructor will initialize the [`tracing::subscriber`] globally.
    ///
    /// ## Errors
    ///
    /// If [`TraceLogger::new`] has already been called.
    pub fn new() -> Result<Self, SetGlobalDefaultError> {
        let subscriber = tracing_subscriber::FmtSubscriber::new();
        tracing::subscriber::set_global_default(subscriber)?;
        Ok(Self {})
    }
}

#[cfg(feature = "trace")]
impl NodeEventHandler for TraceLogger {
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

impl NodeEventHandler for () {
    fn dialog(&self, _dialog: String) {}
    fn warning(&self, _warning: Warning) {}
    fn state_changed(&self, _state: NodeState) {}
    fn connections_met(&self) {}
    fn tx_sent(&self, _txid: Txid) {}
    fn tx_failed(&self, _txid: Txid) {}
    fn blocks_disconnected(&self, _blocks: Vec<u32>) {}
    fn synced(&self, _tip: u32) {}
}
