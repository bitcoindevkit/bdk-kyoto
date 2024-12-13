use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr},
};

use bdk_kyoto::{
    logger::TraceLogger,
    multi::{MultiEventReceiver, MultiSyncRequest},
};
use bdk_wallet::{KeychainKind, Wallet};
use kyoto::{HeaderCheckpoint, Network, NodeBuilder, ScriptBuf};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct WalletId(u8);

const WALLET_ID_ONE: WalletId = WalletId(1);
const WALLET_ID_TWO: WalletId = WalletId(2);

const PRIV_RECV: &str = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/1'/0'/0/*)";
const PRIV_CHANGE: &str = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/1'/0'/1/*)";
const PUB_RECV: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
const PUB_CHANGE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";

const NETWORK: Network = Network::Signet;

const PEER: IpAddr = IpAddr::V4(Ipv4Addr::new(23, 137, 57, 100));
const NUM_PEERS: u8 = 1;

const RECOVERY_HEIGHT: u32 = 170_000;

fn get_scripts_for_wallets(wallets: &[&Wallet]) -> HashSet<ScriptBuf> {
    let mut spks = HashSet::new();
    for wallet in wallets {
        for keychain in [KeychainKind::External, KeychainKind::Internal] {
            let last_revealed = wallet
                .spk_index()
                .last_revealed_index(keychain)
                .unwrap_or(0);
            let lookahead_index = last_revealed + wallet.spk_index().lookahead();
            for index in 0..=lookahead_index {
                spks.insert(wallet.peek_address(keychain, index).script_pubkey());
            }
        }
    }
    spks
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let logger = TraceLogger::new()?;

    let mut wallet_one = Wallet::create(PUB_RECV, PUB_CHANGE)
        .network(NETWORK)
        .lookahead(30)
        .create_wallet_no_persist()?;

    let mut wallet_two = Wallet::create(PRIV_RECV, PRIV_CHANGE)
        .network(NETWORK)
        .lookahead(30)
        .create_wallet_no_persist()?;

    let scripts = get_scripts_for_wallets(&[&wallet_one, &wallet_two]);

    let (node, client) = NodeBuilder::new(NETWORK)
        .add_peer(PEER)
        .num_required_peers(NUM_PEERS)
        .add_scripts(scripts)
        .anchor_checkpoint(HeaderCheckpoint::closest_checkpoint_below_height(
            RECOVERY_HEIGHT,
            NETWORK,
        ))
        .build_node()?;

    tokio::task::spawn(async move { node.run().await });

    let request_one = MultiSyncRequest {
        index: WALLET_ID_ONE,
        checkpoint: wallet_one.local_chain().tip(),
        spk_index: wallet_one.spk_index().clone(),
    };
    let request_two = MultiSyncRequest {
        index: WALLET_ID_TWO,
        checkpoint: wallet_two.local_chain().tip(),
        spk_index: wallet_two.spk_index().clone(),
    };
    let requests = vec![request_one, request_two];

    let (sender, receiver) = client.split();

    let mut event_receiver = MultiEventReceiver::from_requests(requests, receiver)?;
    let updates = event_receiver.updates(&logger).await;
    for (id, update) in updates {
        if id.eq(&WALLET_ID_ONE) {
            wallet_one.apply_update(update)?;
            let balance = wallet_one.balance().total().to_sat();
            tracing::info!("Wallet one has {balance} satoshis");
        } else if id.eq(&WALLET_ID_TWO) {
            wallet_two.apply_update(update)?;
            let balance = wallet_two.balance().total().to_sat();
            tracing::info!("Wallet two has {balance} satoshis");
        }
    }
    sender.shutdown().await?;
    return Ok(());
}
