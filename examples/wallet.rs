#![allow(unused)]

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use bdk_kyoto::builder::{LightClientBuilder, Peer};
use bdk_wallet::bitcoin::{BlockHash, Network, ScriptBuf};
use bdk_wallet::chain::local_chain::CheckPoint;
use bdk_wallet::chain::BlockId;
use bdk_wallet::{
    wallet::{self, Wallet},
    KeychainKind,
};

use kyoto::chain::checkpoints::{HeaderCheckpoint, SIGNET_HEADER_CP};
use kyoto::node::builder::NodeBuilder;
use kyoto::TrustedPeer;

/// Peer address whitelist
const PEERS: &[IpAddr] = &[
    IpAddr::V4(Ipv4Addr::new(170, 75, 163, 219)),
    IpAddr::V4(Ipv4Addr::new(23, 137, 57, 100)),
];

/* Sync a bdk wallet */

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = tracing_subscriber::FmtSubscriber::new();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
    let change_desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";

    // We can use a predefined header if we don't have one. We use 170_000 here
    let (height, hash) = SIGNET_HEADER_CP.into_iter().rev().nth(3).unwrap();
    let header_cp = CheckPoint::new(BlockId {
        height: *height,
        hash: BlockHash::from_str(hash).unwrap(),
    });

    let peers = PEERS
        .into_iter()
        .map(|ip| TrustedPeer::new(*ip, None))
        .collect();

    let mut wallet = Wallet::new(desc, change_desc, Network::Signet)?;

    // The light client builder handles the logic of inserting the SPKs
    let (node, mut client) = LightClientBuilder::new()
        .add_birthday(header_cp)
        .add_peers(peers)
        .build()
        .await;
    client.run_node(node);

    tracing::info!(
        "Balance before sync: {} sats",
        wallet.balance().total().to_sat()
    );

    // Sync and apply updates. We can do this a continual loop while the "application" is running.
    // Often this loop would be on a separate "Task" in a Swift app for instance
    loop {
        if let Some(update) = client.update().await {
            let bdk_kyoto::Update {
                cp,
                indexed_tx_graph,
            } = update;

            wallet.apply_update(wallet::Update {
                chain: Some(cp),
                graph: indexed_tx_graph.graph().clone(),
                last_active_indices: indexed_tx_graph.index.last_used_indices(),
            })?;
            // Do something here to add more scripts?

            let cp = wallet.latest_checkpoint();
            tracing::info!("Tx count: {}", wallet.transactions().count());
            tracing::info!("Balance: {}", wallet.balance().total().to_sat());
            tracing::info!(
                "Last revealed External: {}",
                wallet.derivation_index(KeychainKind::External).unwrap()
            );
            tracing::info!(
                "Last revealed Internal: {}",
                wallet.derivation_index(KeychainKind::Internal).unwrap()
            );
        }
    }
}
