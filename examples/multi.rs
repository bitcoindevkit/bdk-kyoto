use std::net::IpAddr;
use std::net::Ipv4Addr;

use bdk_kyoto::builder::LightClientBuilder;
use bdk_kyoto::MultiLightClient;
use bdk_wallet::Wallet;

use kyoto::Network;

use bdk_kyoto::multi::MultiSyncRequest;
use bdk_kyoto::ScanType;

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = tracing_subscriber::FmtSubscriber::new();
    tracing::subscriber::set_global_default(subscriber)?;

    let mut wallet_one = Wallet::create(PUB_RECV, PUB_CHANGE)
        .network(NETWORK)
        .lookahead(30)
        .create_wallet_no_persist()?;

    let mut wallet_two = Wallet::create(PRIV_RECV, PRIV_CHANGE)
        .network(NETWORK)
        .lookahead(30)
        .create_wallet_no_persist()?;

    let request_one = MultiSyncRequest {
        index: WALLET_ID_ONE,
        scan_type: ScanType::New,
        wallet: &wallet_one,
    };
    let request_two = MultiSyncRequest {
        index: WALLET_ID_TWO,
        scan_type: ScanType::Recovery {
            from_height: RECOVERY_HEIGHT,
        },
        wallet: &wallet_two,
    };
    let requests = vec![request_one, request_two];

    let MultiLightClient {
        requester: _,
        mut log_subscriber,
        mut warning_subscriber,
        mut update_subscriber,
        node,
    } = LightClientBuilder::new()
        .connections(NUM_PEERS)
        .peers(vec![PEER.into()])
        .build_multi(requests)?;

    tokio::task::spawn(async move { node.run().await });

    loop {
        tokio::select! {
            updates = update_subscriber.sync() => {
                for (index, update) in updates {
                    tracing::info!("Got update for wallet {}", index.0);
                    if index == WALLET_ID_ONE {
                        wallet_one.apply_update(update)?;
                        tracing::info!("Wallet one balance {}", wallet_one.balance().total())
                    } else if index == WALLET_ID_TWO {
                        wallet_two.apply_update(update)?;
                        tracing::info!("Wallet two balance {}", wallet_two.balance().total())
                    }
                }
            },
            log = log_subscriber.recv() => {
                if let Some(log) = log {
                    tracing::info!("{log}");
                }
            }
            warn = warning_subscriber.recv() => {
                if let Some(warn) = warn {
                    tracing::warn!("{warn}");
                }
            }
        }
    }
}
