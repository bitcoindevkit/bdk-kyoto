#![allow(unused)]

use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use bdk_chain::bitcoin::constants::genesis_block;
use bdk_chain::bitcoin::secp256k1::Secp256k1;
use bdk_chain::keychain::KeychainTxOutIndex;
use bdk_chain::local_chain::LocalChain;
use bdk_chain::miniscript::Descriptor;
use bdk_chain::{bitcoin, ConfirmationTimeHeightAnchor, IndexedTxGraph};
use bdk_kyoto::Client;
use bdk_wallet::{KeychainKind, Wallet};

use kyoto::chain::checkpoints::HeaderCheckpoint;
use kyoto::node::builder::NodeBuilder;
use kyoto::prelude::bitcoin::BlockHash;
use kyoto::prelude::bitcoin::Network;
use kyoto::prelude::bitcoin::{Address, ScriptBuf};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let secp = Secp256k1::new();

    let desc = std::env::var("DESCRIPTOR")?;
    let (descriptor, _) = Descriptor::parse_descriptor(&secp, &desc)?;
    let desc = std::env::var("CHANGE_DESCRIPTOR")?;
    let (change_descriptor, _) = Descriptor::parse_descriptor(&secp, &desc)?;

    let mut chain: LocalChain = {
        let g = genesis_block(bitcoin::Network::Signet).block_hash();
        let (chain, _) = LocalChain::from_genesis_hash(g);
        chain
    };

    let mut graph: IndexedTxGraph<ConfirmationTimeHeightAnchor, KeychainTxOutIndex<usize>> = {
        let mut index = KeychainTxOutIndex::default();
        let _ = index.insert_descriptor(0, descriptor);
        let _ = index.insert_descriptor(1, change_descriptor);
        IndexedTxGraph::new(index)
    };

    let mut addresses: Vec<Address> = vec![];
    for keychain in 0usize..=1 {
        let (mut iter, _) = graph.index.reveal_to_target(&keychain, 9).unwrap();
        for (idx, spk) in iter {
            let script_bytes = spk.to_bytes();
            let spk = ScriptBuf::from_bytes(script_bytes);
            addresses.push(Address::from_script(&spk, Network::Signet)?);
        }
    }

    let subscriber = tracing_subscriber::FmtSubscriber::new();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let peer = IpAddr::V4(Ipv4Addr::new(170, 75, 163, 219));
    let peer_2 = IpAddr::V4(Ipv4Addr::new(23, 137, 57, 100));

    let builder = NodeBuilder::new(Network::Signet);
    let (mut node, client) = builder
        .add_peers(vec![(peer, 38333), (peer_2, 38333)])
        .add_scripts(addresses)
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

        let graph_cs = indexed_tx_graph.initial_changeset();
        let _ = graph.apply_changeset(graph_cs);
    }

    let cp = chain.tip();
    let index = &graph.index;
    println!("Graph txs count: {}", graph.graph().full_txs().count());
    println!("Local tip: {} {}", cp.height(), cp.hash());
    println!(
        "Balance: {:#?}",
        graph
            .graph()
            .balance(&chain, cp.block_id(), index.outpoints(), |_, _| true)
    );
    println!(
        "Last revealed indices: {:#?}",
        index.last_revealed_indices()
    );

    Ok(())
}
