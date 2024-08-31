use bdk_wallet::chain::Merge;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use tokio::task;

use bdk_kyoto::logger::PrintLogger;
use bdk_kyoto::Client;
use bdk_wallet::bitcoin::{
    constants::genesis_block, secp256k1::Secp256k1, Address, BlockHash, Network, ScriptBuf,
};
use bdk_wallet::chain::{
    keychain_txout::KeychainTxOutIndex, local_chain::LocalChain, miniscript::Descriptor, FullTxOut,
    IndexedTxGraph, SpkIterator,
};
use kyoto::chain::checkpoints::HeaderCheckpoint;
use kyoto::node::builder::NodeBuilder;

const TARGET_INDEX: u32 = 20;

/* Sync bdk chain and txgraph structures */

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let secp = Secp256k1::new();

    let desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
    let change_desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
    let (descriptor, _) = Descriptor::parse_descriptor(&secp, &desc)?;
    let (change_descriptor, _) = Descriptor::parse_descriptor(&secp, &change_desc)?;

    let g = genesis_block(Network::Signet).block_hash();
    let (mut chain, _) = LocalChain::from_genesis_hash(g);

    let mut graph = IndexedTxGraph::new({
        let mut index = KeychainTxOutIndex::default();
        let _ = index.insert_descriptor(0usize, descriptor);
        let _ = index.insert_descriptor(1, change_descriptor);
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
    let (mut node, client) = builder
        .add_peers(peers.into_iter().map(|ip| (ip, None).into()).collect())
        .add_scripts(spks_to_watch)
        .anchor_checkpoint(HeaderCheckpoint::new(
            170_000,
            BlockHash::from_str(
                "00000041c812a89f084f633e4cf47e819a2f6b1c0a15162355a930410522c99d",
            )?,
        ))
        .num_required_peers(2)
        .build_node()
        .unwrap();
    let mut client = Client::from_index(chain.tip(), &graph.index, client).unwrap();

    // Run the node
    if !node.is_running() {
        task::spawn(async move { node.run().await });
    }

    // Sync and apply updates
    let logger = PrintLogger::new();
    if let Some(update) = client.update(&logger).await {
        let _ = chain.apply_update(update.chain_update.unwrap())?;
        let mut indexed_tx_graph_changeset = graph.apply_update(update.tx_update);
        let index_changeset = graph
            .index
            .reveal_to_target_multi(&update.last_active_indices);
        indexed_tx_graph_changeset.merge(index_changeset.into());
        let _ = graph.apply_changeset(indexed_tx_graph_changeset);
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
