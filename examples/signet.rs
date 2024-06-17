#![allow(unused)]

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use bdk_chain::bitcoin::{
    constants::genesis_block, secp256k1::Secp256k1, BlockHash, Network, ScriptBuf,
};
use bdk_chain::keychain::KeychainTxOutIndex;
use bdk_chain::local_chain::LocalChain;
use bdk_chain::miniscript::Descriptor;
use bdk_chain::IndexedTxGraph;

use kyoto::chain::checkpoints::HeaderCheckpoint;
use kyoto::node::builder::NodeBuilder;

// Sync bdk chain and txgraph structures

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let secp = Secp256k1::new();

    let desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
    let change_desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
    let (descriptor, _) = Descriptor::parse_descriptor(&secp, &desc)?;
    let (change_descriptor, _) = Descriptor::parse_descriptor(&secp, &change_desc)?;

    let mut chain: LocalChain = {
        let g = genesis_block(Network::Signet).block_hash();
        let (chain, _) = LocalChain::from_genesis_hash(g);
        chain
    };

    let mut graph = IndexedTxGraph::new({
        let mut index = KeychainTxOutIndex::default();
        let _ = index.insert_descriptor(0usize, descriptor);
        let _ = index.insert_descriptor(1, change_descriptor);
        index
    });

    let mut spks_to_watch: HashSet<ScriptBuf> = HashSet::new();
    for keychain in 0usize..=1 {
        let (indexed_spks, _changeset) = graph.index.reveal_to_target(&keychain, 19).unwrap();
        let mut spks = indexed_spks.into_iter().map(|(_i, spk)| spk);
        spks_to_watch.extend(&mut spks);
    }

    let subscriber = tracing_subscriber::FmtSubscriber::new();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let localhost_v4 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let peer = IpAddr::V4(Ipv4Addr::new(170, 75, 163, 219));
    let peer_2 = IpAddr::V4(Ipv4Addr::new(23, 137, 57, 100));

    let builder = NodeBuilder::new(Network::Signet);
    let (mut node, client) = builder
        .add_peers(vec![(localhost_v4, 38333), (peer, 38333), (peer_2, 38333)])
        .add_scripts(spks_to_watch)
        .anchor_checkpoint(HeaderCheckpoint::new(
            169_000,
            BlockHash::from_str(
                "000000ed6fe89c46140f55ff511c558bcbdb1239ba95474f38f619b3bb657d4a",
            )?,
        ))
        .num_required_peers(2)
        .build_node()
        .await;

    // Start a sync `Request`
    let req = bdk_kyoto::Request::new(chain.tip(), &graph.index);
    let mut client = req.into_client(client);

    // Run the `Node`
    if !node.is_running() {
        tokio::task::spawn(async move { node.run().await });
    }

    // Sync and apply updates
    if let Some(update) = client.sync().await {
        let bdk_kyoto::Update {
            cp,
            indexed_tx_graph,
        } = update;

        let _ = chain.apply_update(cp)?;

        let graph_changeset = indexed_tx_graph.initial_changeset();
        let _ = graph.apply_changeset(graph_changeset);
    }

    // Shutdown
    client.shutdown().await?;

    let cp = chain.tip();
    let index = &graph.index;
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
