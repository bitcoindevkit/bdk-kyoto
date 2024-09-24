use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use tokio::task;

use bdk_chain::bitcoin::{
    constants::genesis_block, secp256k1::Secp256k1, Address, BlockHash, Network, ScriptBuf,
};
use bdk_chain::{
    keychain_txout::KeychainTxOutIndex, local_chain::LocalChain, miniscript::Descriptor, FullTxOut,
    IndexedTxGraph, SpkIterator,
};
use bdk_kyoto::logger::PrintLogger;
use bdk_kyoto::Client;
use kyoto::chain::checkpoints::HeaderCheckpoint;
use kyoto::core::builder::NodeBuilder;

const TARGET_INDEX: u32 = 20;

/* Sync bdk chain and txgraph structures */

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let secp = Secp256k1::new();

    let desc = "tr([83737d5e/86'/1'/0']tpubDDR5GgtoxS8fJyjjvdahN4VzV5DV6jtbcyvVXhEKq2XtpxjxBXmxH3r8QrNbQqHg4bJM1EGkxi7Pjfkgnui9jQWqS7kxHvX6rhUeriLDKxz/0/*)";
    let change_desc = "tr([83737d5e/86'/1'/0']tpubDDR5GgtoxS8fJyjjvdahN4VzV5DV6jtbcyvVXhEKq2XtpxjxBXmxH3r8QrNbQqHg4bJM1EGkxi7Pjfkgnui9jQWqS7kxHvX6rhUeriLDKxz/1/*)";
    let (descriptor, _) = Descriptor::parse_descriptor(&secp, desc)?;
    let (change_descriptor, _) = Descriptor::parse_descriptor(&secp, change_desc)?;

    let genesis_hash = genesis_block(Network::Signet).block_hash();
    let (mut chain, _) = LocalChain::from_genesis_hash(genesis_hash);

    let mut graph = IndexedTxGraph::new({
        let mut index = KeychainTxOutIndex::default();
        let _ = index.insert_descriptor("external", descriptor);
        let _ = index.insert_descriptor("internal", change_descriptor);
        index
    });

    let mut spks_to_watch: HashSet<ScriptBuf> = HashSet::new();
    for (_k, desc) in graph.index.keychains() {
        for (_i, spk) in SpkIterator::new_with_range(desc, 0..TARGET_INDEX) {
            spks_to_watch.insert(spk);
        }
    }

    let peers = vec![
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(170, 75, 163, 219)),
        IpAddr::V4(Ipv4Addr::new(23, 137, 57, 100)),
    ];

    let builder = NodeBuilder::new(Network::Signet);
    let (node, client) = builder
        .add_peers(peers.into_iter().map(|ip| ip.into()).collect())
        .add_scripts(spks_to_watch)
        .anchor_checkpoint(HeaderCheckpoint::new(
            205_000,
            BlockHash::from_str(
                "0000002bd0f82f8c0c0f1e19128f84c938763641dba85c44bdb6aed1678d16cb",
            )?,
        ))
        .num_required_peers(2)
        .build_node()?;
    let mut client = Client::from_index(chain.tip(), &graph.index, client)?;

    // Run the node
    if !node.is_running() {
        task::spawn(async move { node.run().await });
    }

    // Sync and apply updates
    let logger = PrintLogger::new();
    if let Some(update) = client.update(&logger).await {
        let _ = chain.apply_update(update.chain_update.unwrap())?;
        let _ = graph.apply_update(update.tx_update);
        let _ = graph
            .index
            .reveal_to_target_multi(&update.last_active_indices);
    }

    // Shutdown
    client.shutdown().await?;

    let cp = chain.tip();
    let index = &graph.index;
    let outpoints = index.outpoints().clone();
    let unspent: Vec<FullTxOut<_>> = graph
        .graph()
        .filter_chain_unspents(&chain, cp.block_id(), outpoints)
        .map(|(_, txout)| txout)
        .collect();
    for utxo in unspent {
        let addr = Address::from_script(utxo.txout.script_pubkey.as_script(), Network::Signet)?;
        println!("Funded: {:?}", addr);
    }
    println!("Graph txs count: {}", graph.graph().full_txs().count());
    println!("Local tip: {} {}", cp.height(), cp.hash());
    println!(
        "Balance: {:#?}",
        graph.graph().balance(
            &chain,
            cp.block_id(),
            index.outpoints().iter().cloned(),
            |_, _| true
        )
    );
    println!(
        "Last revealed indices: {:#?}",
        index.last_revealed_indices()
    );

    Ok(())
}
