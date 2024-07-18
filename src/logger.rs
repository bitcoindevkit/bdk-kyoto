//! Handle messages from a node

use std::fmt::Debug;

pub use kyoto::node::node::NodeState;
pub use kyoto::node::messages::Warning;

/// Handle dialog and state changes from a node with some arbitrary behavior
pub trait NodeMessageHandler: Send + Sync + Debug +'static {
    /// Make use of some message the node has sent.
    fn handle_dialog(&self, dialog: String);
    /// Make use of some warning the ndoe has sent.
    fn handle_warning(&self, warning: Warning);
    /// Handle a change in the node's state.
    fn handle_state_change(&self, state: NodeState);
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
    fn handle_dialog(&self, dialog: String) {
        println!("{dialog}")
    }

    fn handle_warning(&self, warning: Warning) {
        println!("{warning}")
    }

    fn handle_state_change(&self, state: NodeState) {
        println!("State change: {state:?}")
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
    fn handle_dialog(&self, dialog: String) {
        tracing::info!("{dialog}")
    }

    fn handle_warning(&self, warning: Warning) {
        tracing::warn!("{warning}")
    }

    fn handle_state_change(&self, state: NodeState) {
        tracing::info!("State change: {state:?}")
    }
}

impl NodeMessageHandler for () {
    fn handle_dialog(&self, _dialog: String) {}
    fn handle_warning(&self, _warning: Warning) {}
    fn handle_state_change(&self, _state: NodeState) {}
}

