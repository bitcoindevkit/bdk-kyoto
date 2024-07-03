//! [`bdk_kyoto::Client`] builder

use std::{collections::HashSet, net::IpAddr};

use bdk_wallet::{
    chain::{local_chain::CheckPoint, BlockId},
    KeychainKind, Wallet,
};
use kyoto::{
    chain::checkpoints::HeaderCheckpoint,
    node::{builder::NodeBuilder, node::Node},
    ScriptBuf,
};

use crate::{Client, Request};

// There is very little cost to doing a lookahead this generous.
// By doing so, a user can reveal many scripts without ever having
// to add them on the fly to the node.
const TARGET_INDEX: u32 = 100;

#[derive(Debug)]
/// Construct a light client from higher level components.
pub struct LightClientBuilder<'a> {
    wallet: &'a Wallet,
    peers: Option<Vec<Peer>>,
    required_peers: Option<u8>,
    birthday: Option<CheckPoint>,
}

impl<'a> LightClientBuilder<'a> {
    /// Construct a new node builder
    pub fn new(wallet: &'a Wallet) -> Self {
        Self {
            wallet,
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
    pub fn add_peers(mut self, peers: Vec<Peer>) -> Self {
        self.peers = Some(peers);
        self
    }

    /// Build a light client node and a client to interact with the node
    pub async fn build(self) -> (Node, Client<KeychainKind>) {
        let mut node_builder = NodeBuilder::new(self.wallet.network());
        if let Some(whitelist) = self.peers {
            let peers = whitelist.iter().map(|peer| (peer.0, peer.1)).collect();
            node_builder = node_builder.add_peers(peers);
        }
        match self.birthday {
            Some(birthday) => {
                if birthday.height() < self.wallet.local_chain().tip().height() {
                    let block_id = self.wallet.local_chain().tip();
                    let header_cp = HeaderCheckpoint::new(block_id.height(), block_id.hash());
                    node_builder = node_builder.anchor_checkpoint(header_cp)
                } else {
                    node_builder = node_builder.anchor_checkpoint(HeaderCheckpoint::new(
                        birthday.height(),
                        birthday.hash(),
                    ))
                }
            }
            None => {
                let block_id = self.wallet.local_chain().tip();
                let header_cp = HeaderCheckpoint::new(block_id.height(), block_id.hash());
                node_builder = node_builder.anchor_checkpoint(header_cp)
            }
        }
        node_builder = node_builder.num_required_peers(self.required_peers.unwrap_or(2));
        let mut spks: HashSet<ScriptBuf> = HashSet::new();
        for keychain in [KeychainKind::External, KeychainKind::Internal] {
            for index in 0..=TARGET_INDEX {
                spks.insert(self.wallet.peek_address(keychain, index).script_pubkey());
            }
        }
        let (node, kyoto_client) = node_builder.add_scripts(spks).build_node().await;
        let request = Request::new(self.wallet.local_chain().tip(), self.wallet.spk_index());
        let client = request.into_client(kyoto_client);
        (node, client)
    }
}

#[derive(Debug, Clone, Copy)]
/// A peer to connect to on the Bitcoin P2P network
pub struct Peer(pub IpAddr, pub u16);
