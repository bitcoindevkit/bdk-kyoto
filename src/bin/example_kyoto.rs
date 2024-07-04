// #![allow(unused)]

// use std::net::{IpAddr, Ipv4Addr};
// use std::str::FromStr;
// use std::sync::Mutex;
// use tokio::task;
// use tokio::time;

// use bdk_wallet::bitcoin::{constants, Network};

// use bdk_wallet::chain::{
//     collections::HashSet,
//     indexed_tx_graph::{self, IndexedTxGraph},
//     keychain,
//     local_chain::{self, CheckPoint, LocalChain},
//     BlockId, ConfirmationTimeHeightAnchor,
// };

// use example_cli::{
//     clap::{self, Args, Subcommand},
//     Keychain,
// };

// use kyoto::chain::checkpoints::HeaderCheckpoint;
// use kyoto::node::builder::NodeBuilder;
// use kyoto::BlockHash;

// type ChangeSet = (
//     local_chain::ChangeSet,
//     indexed_tx_graph::ChangeSet<ConfirmationTimeHeightAnchor, keychain::ChangeSet<Keychain>>,
// );

// /// Peer address whitelist
// const PEERS: &[IpAddr] = &[
//     IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
//     IpAddr::V4(Ipv4Addr::new(170, 75, 163, 219)),
//     IpAddr::V4(Ipv4Addr::new(23, 137, 57, 100)),
// ];
// /// Bitcoin P2P port
// const PORT: u16 = 38333;
// /// Target derivation index in case none are revealed for a keychain.
// const TARGET_INDEX: u32 = 20;

// const DB_MAGIC: &[u8] = b"bdk_kyoto_example";
// const DB_PATH: &str = ".bdk_kyoto_example.db";

// #[derive(Debug, Clone, Subcommand)]
// enum Cmd {
//     /// Sync
//     Sync {
//         #[clap(flatten)]
//         args: Arg,
//     },
// }

// #[derive(Args, Debug, Clone)]
// struct Arg {
//     /// Start height
//     #[clap(env = "START_HEIGHT")]
//     height: u32,
//     /// Start hash
//     #[clap(env = "START_HASH")]
//     hash: BlockHash,
// }

// #[tokio::main]
// async fn main() -> anyhow::Result<()> {
//     let subscriber = tracing_subscriber::FmtSubscriber::new();
//     tracing::subscriber::set_global_default(subscriber)?;

//     let example_cli::Init {
//         args,
//         keymap,
//         index,
//         db,
//         init_changeset,
//     } = example_cli::init::<Cmd, Arg, ChangeSet>(DB_MAGIC, DB_PATH)?;

//     let (init_chain_changeset, init_indexed_tx_graph_changeset) = init_changeset;

//     let graph = Mutex::new({
//         let mut graph = IndexedTxGraph::new(index);
//         graph.apply_changeset(init_indexed_tx_graph_changeset);
//         graph
//     });

//     let (chain, local_heights) = {
//         let g = constants::genesis_block(args.network).block_hash();
//         let (mut chain, _) = LocalChain::from_genesis_hash(g);
//         chain.apply_changeset(&init_chain_changeset)?;
//         let heights: HashSet<_> = chain.iter_checkpoints().map(|cp| cp.height()).collect();
//         (Mutex::new(chain), heights)
//     };

//     let cmd = match &args.command {
//         example_cli::Commands::ChainSpecific(cmd) => cmd,
//         general_cmd => {
//             return example_cli::handle_commands(
//                 &graph,
//                 &db,
//                 &chain,
//                 &keymap,
//                 args.network,
//                 |_, _tx| unimplemented!(),
//                 general_cmd.clone(),
//             );
//         }
//     };

//     let now = time::Instant::now();

//     match cmd {
//         Cmd::Sync { args } => {
//             let mut spks = HashSet::new();

//             let header_cp = {
//                 let chain = chain.lock().unwrap();
//                 let graph = graph.lock().unwrap();

//                 // Populate list of watched SPKs
//                 let indexer = &graph.index;
//                 for (keychain, _) in indexer.keychains() {
//                     let last_reveal = indexer
//                         .last_revealed_index(keychain)
//                         .unwrap_or(TARGET_INDEX);
//                     for index in 0..=last_reveal {
//                         let spk = graph.index.spk_at_index(*keychain, index).unwrap();
//                         spks.insert(spk.to_owned());
//                     }
//                 }

//                 // Begin sync from at least the specified wallet birthday
//                 // or else the last local checkpoint
//                 let cp = chain.tip();
//                 if cp.height() < args.height {
//                     HeaderCheckpoint::new(args.height, args.hash)
//                 } else {
//                     HeaderCheckpoint::new(cp.height(), cp.hash())
//                 }
//             };

//             // Configure kyoto node
//             let builder = NodeBuilder::new(Network::Signet);
//             let (mut node, client) = builder
//                 .add_peers(
//                     PEERS
//                         .iter()
//                         .cloned()
//                         .map(|ip| (ip, Some(PORT)).into())
//                         .collect(),
//                 )
//                 .add_scripts(spks)
//                 .anchor_checkpoint(header_cp)
//                 .num_required_peers(2)
//                 .build_node()
//                 .await;

//             let mut client = {
//                 let chain = chain.lock().unwrap();
//                 let graph = graph.lock().unwrap();

//                 let req = bdk_kyoto::Request::new(chain.tip(), &graph.index);
//                 req.into_client(client)
//             };

//             // Run the `Node`
//             if !node.is_running() {
//                 task::spawn(async move { node.run().await });
//             }

//             // Sync and apply updates
//             if let Some(bdk_kyoto::Update {
//                 cp,
//                 indexed_tx_graph,
//             }) = client.update().await
//             {
//                 let mut chain = chain.lock().unwrap();
//                 let mut graph = graph.lock().unwrap();
//                 let mut db = db.lock().unwrap();

//                 let chain_changeset = chain.apply_update(cp)?;
//                 let graph_changeset = indexed_tx_graph.initial_changeset();
//                 graph.apply_changeset(graph_changeset.clone());
//                 db.append_changeset(&(chain_changeset, graph_changeset))?;
//             }

//             client.shutdown().await?;
//         }
//     }

//     let elapsed = now.elapsed();
//     println!("Duration: {}s", elapsed.as_secs_f32());

//     let chain = chain.lock().unwrap();
//     let graph = graph.lock().unwrap();
//     let cp = chain.tip();
//     println!("Local tip: {} {}", cp.height(), cp.hash());
//     let outpoints = graph.index.outpoints().clone();
//     println!(
//         "Balance: {:#?}",
//         graph
//             .graph()
//             .balance(&*chain, cp.block_id(), outpoints, |_, _| true)
//     );
//     let new_heights: HashSet<_> = chain.iter_checkpoints().map(|cp| cp.height()).collect();
//     assert!(
//         new_heights.is_superset(&local_heights),
//         "all heights in original chain must be present"
//     );

//     Ok(())
// }

fn main() {}
