//! [`bdk_kyoto::Client`] builder

use std::{collections::HashSet, net::IpAddr, str::FromStr};

use bdk_wallet::{
    chain::{keychain::KeychainTxOutIndex, local_chain::CheckPoint, BlockId},
    KeychainKind, Wallet,
};
use kyoto::{
    chain::checkpoints::HeaderCheckpoint, node::{builder::NodeBuilder, node::Node}, BlockHash, ScriptBuf, TrustedPeer
};

use crate::{Client, Request};

// There is very little cost to doing a lookahead this generous.
// By doing so, a user can reveal many scripts without ever having
// to add them on the fly to the node.
const TARGET_INDEX: u32 = 100;

#[derive(Debug)]
/// Construct a light client from higher level components.
pub struct LightClientBuilder {
    peers: Option<Vec<TrustedPeer>>,
    required_peers: Option<u8>,
    birthday: Option<CheckPoint>,
}

impl LightClientBuilder {
    /// Construct a new node builder
    pub fn new() -> Self {
        Self {
            peers: None,
            required_peers: None,
            birthday: None,
        }
    }

    /// Add a wallet "birthday", or block to start searching for transactions _strictly after_.
    pub fn add_birthday(mut self, birthday: CheckPoint) -> Self {
        self.birthday = Some(birthday);
        self
    }

    /// Add peers to connect to over the P2P network.
    pub fn add_peers(mut self, peers: Vec<TrustedPeer>) -> Self {
        self.peers = Some(peers);
        self
    }

    /// Build a light client node and a client to interact with the node
    pub async fn build(self) -> (Node, Client<KeychainKind>) {
        let mut node_builder = NodeBuilder::new(kyoto::Network::Signet);
        if let Some(whitelist) = self.peers {
            node_builder = node_builder.add_peers(whitelist);
        }
        let cp = HeaderCheckpoint::new(170_000, BlockHash::from_str("00000041c812a89f084f633e4cf47e819a2f6b1c0a15162355a930410522c99d").unwrap());
        let checkpoint = CheckPoint::new(BlockId { height: cp.height, hash: cp.hash });
        node_builder = node_builder.num_required_peers(self.required_peers.unwrap_or(2));
        let (node, kyoto_client) = node_builder.build_node().await;
        let tx_index = &KeychainTxOutIndex::new(10);
        let request = Request::new(checkpoint, tx_index);
        let client = request.into_client(kyoto_client);
        (node, client)
    }
}

#[derive(Debug, Clone, Copy)]
/// A peer to connect to on the Bitcoin P2P network
pub struct Peer(pub IpAddr, pub u16);
