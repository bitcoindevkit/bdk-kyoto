#![allow(unused)]

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use bdk_wallet::bitcoin::{BlockHash, Network, ScriptBuf};
use bdk_wallet::{
    wallet::{self, Wallet},
    KeychainKind,
};

use kyoto::chain::checkpoints::HeaderCheckpoint;
use kyoto::node::builder::NodeBuilder;

/* Sync a bdk wallet */

// Highest derivation index to include in the sync
const TARGET_INDEX: u32 = 19;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
    let change_desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";

    let mut wallet = Wallet::new(desc, change_desc, Network::Signet)?;

    println!(
        "Balance before sync: {} sats",
        wallet.balance().total().to_sat()
    );

    // Populate the list of watched script pubkeys
    let mut spks: HashSet<ScriptBuf> = HashSet::new();
    for keychain in [KeychainKind::External, KeychainKind::Internal] {
        for index in 0..=TARGET_INDEX {
            spks.insert(wallet.peek_address(keychain, index).script_pubkey());
        }
    }

    // Set a wallet birthday
    let header_cp = HeaderCheckpoint::new(
        169_000,
        BlockHash::from_str("000000ed6fe89c46140f55ff511c558bcbdb1239ba95474f38f619b3bb657d4a")?,
    );

    // Configure kyoto node and console logger
    let subscriber = tracing_subscriber::FmtSubscriber::new();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let port = 38333;
    let peers = vec![
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(170, 75, 163, 219)),
        IpAddr::V4(Ipv4Addr::new(23, 137, 57, 100)),
    ];
    let builder = NodeBuilder::new(Network::Signet);
    let (mut node, client) = builder
        .add_peers(peers.into_iter().map(|ip| (ip, port)).collect())
        .add_scripts(spks)
        .anchor_checkpoint(header_cp)
        .num_required_peers(2)
        .build_node()
        .await;

    // Start a sync request
    let req = bdk_kyoto::Request::new(wallet.local_chain().tip(), wallet.spk_index());
    let mut client = req.into_client(client);

    // Run the node
    if !node.is_running() {
        tokio::task::spawn(async move { node.run().await });
    }

    // Sync and apply updates
    if let Some(update) = client.sync().await {
        let bdk_kyoto::Update {
            cp,
            indexed_tx_graph,
        } = update;

        wallet.apply_update(wallet::Update {
            chain: Some(cp),
            graph: indexed_tx_graph.graph().clone(),
            last_active_indices: indexed_tx_graph.index.last_used_indices(),
        })?;
    }

    let _ = client.shutdown().await?;

    let cp = wallet.latest_checkpoint();
    println!("Synced to tip: {} {}", cp.height(), cp.hash());
    println!("Tx count: {}", wallet.transactions().count());
    println!("Balance: {:#?}", wallet.balance());
    println!(
        "Last revealed External: {}",
        wallet.derivation_index(KeychainKind::External).unwrap()
    );
    println!(
        "Last revealed Internal: {}",
        wallet.derivation_index(KeychainKind::Internal).unwrap()
    );

    Ok(())
}
