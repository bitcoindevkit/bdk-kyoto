//! [`bdk_kyoto::Client`] builder

use std::{collections::HashSet, path::PathBuf, str::FromStr, sync::Arc};

use bdk_wallet::{KeychainKind, Wallet};
use kyoto::{
    chain::checkpoints::{
        HeaderCheckpoint, MAINNET_HEADER_CP, REGTEST_HEADER_CP, SIGNET_HEADER_CP,
    },
    node::{builder::NodeBuilder, node::Node},
    BlockHash, Network, ScriptBuf, TrustedPeer,
};

use crate::{logger::NodeMessageHandler, Client};

const TARGET_INDEX: u32 = 20;
const RECOMMENDED_PEERS: u8 = 2;

#[derive(Debug)]
/// Construct a light client from higher level components.
pub struct LightClientBuilder<'a> {
    wallet: &'a Wallet,
    peers: Option<Vec<TrustedPeer>>,
    connections: Option<u8>,
    birthday_height: Option<u32>,
    data_dir: Option<PathBuf>,
    message_handler: Option<Arc<dyn NodeMessageHandler>>,
}

impl<'a> LightClientBuilder<'a> {
    /// Construct a new node builder
    pub fn new(wallet: &'a Wallet) -> Self {
        Self {
            wallet,
            peers: None,
            connections: None,
            birthday_height: None,
            data_dir: None,
            message_handler: None,
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

    /// Handle messages from the node
    pub fn logger(mut self, message_handler: Arc<dyn NodeMessageHandler>) -> Self {
        self.message_handler = Some(message_handler);
        self
    }

    /// Add a directory to store node data
    pub fn data_dir(mut self, dir: PathBuf) -> Self {
        self.data_dir = Some(dir);
        self
    }

    /// Add a wallet "birthday", or block to start searching for transactions _strictly after_.
    /// Only useful for recovering wallets. If the wallet has a tip that is already higher than the
    /// height provided, this height will be ignored.
    pub fn scan_after(mut self, height: u32) -> Self {
        self.birthday_height = Some(height);
        self
    }

    // Get the most recent checkpoint that is less than the recovery height.
    fn get_checkpoint_for_height(height: u32, network: &Network) -> HeaderCheckpoint {
        let checkpoints: Vec<HeaderCheckpoint> = match network {
            Network::Bitcoin => MAINNET_HEADER_CP
                .iter()
                .copied()
                .map(|(height, hash)| {
                    HeaderCheckpoint::new(height, BlockHash::from_str(hash).unwrap())
                })
                .collect(),
            Network::Testnet => panic!(),
            Network::Signet => SIGNET_HEADER_CP
                .iter()
                .copied()
                .map(|(height, hash)| {
                    HeaderCheckpoint::new(height, BlockHash::from_str(hash).unwrap())
                })
                .collect(),
            Network::Regtest => REGTEST_HEADER_CP
                .iter()
                .copied()
                .map(|(height, hash)| {
                    HeaderCheckpoint::new(height, BlockHash::from_str(hash).unwrap())
                })
                .collect(),
            _ => unreachable!(),
        };
        let mut cp = *checkpoints.first().unwrap();
        for checkpoint in checkpoints {
            if height.ge(&checkpoint.height) {
                cp = checkpoint;
            } else {
                break;
            }
        }
        cp
    }

    /// Build a light client node and a client to interact with the node
    pub fn build(self) -> (Node, Client<KeychainKind>) {
        let network = self.wallet.network();
        let mut node_builder = NodeBuilder::new(network);
        if let Some(whitelist) = self.peers {
            node_builder = node_builder.add_peers(whitelist);
        }
        match self.birthday_height {
            Some(birthday) => {
                // If there is a birthday at a height less than our local chain, we may assume we've already synced
                // the wallet past the birthday height and no longer need it.
                if birthday < self.wallet.local_chain().tip().height() {
                    let block_id = self.wallet.local_chain().tip();
                    let header_cp = HeaderCheckpoint::new(block_id.height(), block_id.hash());
                    node_builder = node_builder.anchor_checkpoint(header_cp)
                } else {
                    let cp = Self::get_checkpoint_for_height(birthday, &network);
                    node_builder = node_builder.anchor_checkpoint(cp)
                }
            }
            None => {
                // If there is no birthday provided and the local chain starts at the genesis block, we assume this
                // is a new wallet and use the most recent checkpoint. Otherwise we sync from the last known tip in the
                // LocalChain.
                let block_id = self.wallet.local_chain().tip();
                if block_id.height() > 0 {
                    let header_cp = HeaderCheckpoint::new(block_id.height(), block_id.hash());
                    node_builder = node_builder.anchor_checkpoint(header_cp)
                }
            }
        }
        if let Some(dir) = self.data_dir {
            node_builder = node_builder.add_data_dir(dir);
        }
        node_builder =
            node_builder.num_required_peers(self.connections.unwrap_or(RECOMMENDED_PEERS));
        let mut spks: HashSet<ScriptBuf> = HashSet::new();
        // Reveal 20 scripts ahead of the last revealed index so we don't miss any transactions.
        for keychain in [KeychainKind::External, KeychainKind::Internal] {
            for index in 0..=self
                .wallet
                .spk_index()
                .last_revealed_index(keychain)
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
