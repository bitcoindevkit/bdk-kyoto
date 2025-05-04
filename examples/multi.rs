use bdk_kyoto::multi::WalletId;
use bdk_kyoto::{Client, NodeBuilderExt};
use bdk_wallet::Wallet;

use kyoto::{Network, Receiver, UnboundedReceiver, Info, Warning};

use bdk_kyoto::multi::MultiSyncRequest;
use bdk_kyoto::ScanType;
use kyoto::NodeBuilder;
use tokio::select;

const PRIV_RECV: &str = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/1'/0'/0/*)";
const PRIV_CHANGE: &str = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/1'/0'/1/*)";
const PUB_RECV: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
const PUB_CHANGE: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";

const NETWORK: Network = Network::Signet;

const NUM_PEERS: u8 = 1;

const RECOVERY_HEIGHT: u32 = 190_000;

async fn log(
    mut log_rx: Receiver<String>,
    mut info_rx: Receiver<Info>,
    mut warn_rx: UnboundedReceiver<Warning>,
) {
    loop {
        select! {
            log = log_rx.recv() => {
                if let Some(log) = log {
                    tracing::info!("{log}");
                }
            },
            info = info_rx.recv() => {
                if let Some(info) = info {
                    tracing::info!("{info}")
                }
            }
            warn = warn_rx.recv() => {
                if let Some(warn) = warn {
                    tracing::warn!("{warn}");
                }
            }
        }
    }
}

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
        index: WalletId::ONE,
        scan_type: ScanType::New,
        wallet: &wallet_one,
    };
    let request_two = MultiSyncRequest {
        index: WalletId::TWO,
        scan_type: ScanType::Recovery {
            from_height: RECOVERY_HEIGHT,
        },
        wallet: &wallet_two,
    };
    let requests = vec![request_one, request_two];
    let (client, mut update_subscriber, node) = NodeBuilder::new(NETWORK)
        .required_peers(NUM_PEERS)
        .build_multi(requests.into_iter())?;
    let Client {
        log_subscriber,
        info_subscriber,
        warning_subscriber,
        ..
    } = client;

    tokio::task::spawn(async move {
        if let Err(e) = node.run().await {
            tracing::warn!("{e}")
        }
    });
    tokio::task::spawn(async move {
        log(log_subscriber, info_subscriber, warning_subscriber).await;
        tracing::warn!("Unexpected return from logger")
    });

    loop {
        let updates = update_subscriber.sync().await;
        for (id, update) in updates {
            match id {
                WalletId::ONE => {
                    wallet_one.apply_update(update)?;
                    tracing::info!("Wallet one balance {}", wallet_one.balance().total());
                }
                WalletId::TWO => {
                    wallet_two.apply_update(update)?;
                    tracing::info!("Wallet two balance {}", wallet_two.balance().total());
                }
                _ => (),
            }
        }
    }
}
