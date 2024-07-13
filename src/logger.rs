//! Handle messages from a node

use std::fmt::Debug;

pub use kyoto::node::node::NodeState;

/// Handle dialog and state changes from a node with some arbitrary behavior
pub trait NodeMessageHandler {
    /// Make use of some message the node has sent.
    fn handle_dialog(&self, dialog: String);
    /// Make use of some warning the ndoe has sent.
    fn handle_warning(&self, warning: String);
    /// Handle a change in the node's state.
    fn handle_state_change(&self, state: NodeState);
}

/// Print messages from the node to the console
#[derive(Default)]
pub struct PrintLogger {}

impl PrintLogger {
    /// Build a new print logger
    pub fn new() -> Self {
        Self {}
    }
}

impl NodeMessageHandler for PrintLogger {
    fn handle_dialog(&self, dialog: String) {
        println!("{dialog}")
    }

    fn handle_warning(&self, warning: String) {
        println!("{warning}")
    }

    fn handle_state_change(&self, state: NodeState) {
        println!("State change: {state:?}")
    }
}

/// Print messages from the node to the console
#[cfg(feature = "trace")]
#[derive(Default)]
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
    fn handle_dialog(&self, dialog: String) {
        tracing::info!("{dialog}")
    }

    fn handle_warning(&self, warning: String) {
        tracing::warn!("{warning}")
    }

    fn handle_state_change(&self, state: NodeState) {
        tracing::info!("State change: {state:?}")
    }
}

impl Debug for dyn NodeMessageHandler + Send + Sync + 'static {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(write!(f, "Node message handler")?)
    }
}
