//! [`bdk_kyoto::Client`] builder

use std::collections::HashSet;

use bdk_wallet::{chain::local_chain::CheckPoint, KeychainKind, Wallet};
use kyoto::{
    chain::checkpoints::HeaderCheckpoint,
    node::{builder::NodeBuilder, node::Node},
    ScriptBuf, TrustedPeer,
};

use crate::{handler::{NodeMessageHandler, PrintLogger}, Client};

const TARGET_INDEX: u32 = 20;
const RECOMMENDED_PEERS: u8 = 2;

#[derive(Debug)]
/// Construct a light client from higher level components.
pub struct LightClientBuilder<'a> {
    wallet: &'a Wallet,
    peers: Option<Vec<TrustedPeer>>,
    connections: Option<u8>,
    birthday: Option<CheckPoint>,
    message_handler: Option<Box<dyn NodeMessageHandler + Send + Sync + 'static>>
}

impl<'a> LightClientBuilder<'a> {
    /// Construct a new node builder
    pub fn new(wallet: &'a Wallet) -> Self {
        Self {
            wallet,
            peers: None,
            connections: None,
            birthday: None,
            message_handler: None,
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

    /// Add the number of connections for the node to maintain.
    pub fn connections(mut self, num_connections: u8) -> Self {
        self.connections = Some(num_connections);
        self
    }

    /// Handle messages from the node
    pub fn logger(mut self, message_handler: impl NodeMessageHandler + Send + Sync + 'static) -> Self {
        self.message_handler = Some(Box::new(message_handler));
        self
    }

    /// Build a light client node and a client to interact with the node
    pub fn build(self) -> (Node, Client<KeychainKind>) {
        let mut node_builder = NodeBuilder::new(self.wallet.network());
        if let Some(whitelist) = self.peers {
            node_builder = node_builder.add_peers(whitelist);
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
        node_builder =
            node_builder.num_required_peers(self.connections.unwrap_or(RECOMMENDED_PEERS));
        let mut spks: HashSet<ScriptBuf> = HashSet::new();
        for keychain in [KeychainKind::External, KeychainKind::Internal] {
            for index in 0..=self
                .wallet
                .spk_index()
                .last_revealed_index(&keychain)
                .unwrap_or(0)
                + TARGET_INDEX
            {
                spks.insert(self.wallet.peek_address(keychain, index).script_pubkey());
            }
        }
        let (node, kyoto_client) = node_builder.add_scripts(spks).build_node();
        let mut client = Client::from_index(
            self.wallet.local_chain().tip(),
            self.wallet.spk_index(),
            kyoto_client,
        );
        if let Some(logger) = self.message_handler {
            client.set_logger(logger)
        }
        (node, client)
    }
}
