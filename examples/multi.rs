use bdk_kyoto::builder::{Builder, BuilderExt};
use bdk_kyoto::{Info, Receiver, ScanType, UnboundedReceiver, Warning};
use bdk_wallet::bitcoin::Network;
use bdk_wallet::chain::DescriptorExt;
use bdk_wallet::{KeychainKind, Wallet};
use tokio::select;

const RECV_ONE: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/0/*)";
const CHANGE_ONE: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/1/*)";
const RECV_TWO: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
const CHANGE_TWO: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
const NETWORK: Network = Network::Signet;

/* Sync a bdk wallet */

async fn traces(
    mut info_subscriber: Receiver<Info>,
    mut warning_subscriber: UnboundedReceiver<Warning>,
) {
    loop {
        select! {
            info = info_subscriber.recv() => {
                if let Some(info) = info {
                    match info {
                        Info::Progress(p) => {
                            tracing::info!("chain height: {}, filter download progress: {}%", p.chain_height(), p.percentage_complete());
                        },
                        Info::BlockReceived(b) => {
                            tracing::info!("downloaded block: {b}");
                        },
                        _ => (),
                    }
                }
            }
            warn = warning_subscriber.recv() => {
                if let Some(warn) = warn {
                    tracing::warn!("{warn}")
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = tracing_subscriber::FmtSubscriber::new();
    tracing::subscriber::set_global_default(subscriber)?;

    let mut wallet_one = Wallet::create(RECV_ONE, CHANGE_ONE)
        .network(NETWORK)
        .create_wallet_no_persist()?;

    let mut wallet_two = Wallet::create(RECV_TWO, CHANGE_TWO)
        .network(NETWORK)
        .create_wallet_no_persist()?;

    let wallet_iter = vec![(&wallet_one, ScanType::Sync), (&wallet_two, ScanType::Sync)];

    // The light client builder handles the logic of inserting the SPKs
    let client = Builder::new(NETWORK)
        .build_with_wallets(wallet_iter)
        .unwrap();
    let (client, logging, mut update_subscriber) = client.subscribe();
    tokio::task::spawn(
        async move { traces(logging.info_subscriber, logging.warning_subscriber).await },
    );
    let client = client.start();
    let requester = client.requester();

    // Sync and apply updates. We can do this in a continual loop while the "application" is running.
    // Often this would occur on a separate thread than the underlying application user interface.
    loop {
        let updates = update_subscriber.updates().await?;
        for (desc_id, update) in updates {
            if wallet_one
                .public_descriptor(KeychainKind::External)
                .descriptor_id()
                .eq(&desc_id)
            {
                wallet_one.apply_update(update)?;
                tracing::info!("Wallet one summary: ");
                tracing::info!("Balance: {:#}", wallet_one.balance().total());
                tracing::info!(
                    "Local chain tip: {}",
                    wallet_one.local_chain().tip().height()
                );
            } else if wallet_two
                .public_descriptor(KeychainKind::External)
                .descriptor_id()
                .eq(&desc_id)
            {
                wallet_two.apply_update(update)?;
                tracing::info!("Wallet two summary: ");
                tracing::info!("Balance: {:#}", wallet_two.balance().total());
                tracing::info!(
                    "Local chain tip: {}",
                    wallet_two.local_chain().tip().height()
                );
            }
        }
        let fee_filter = requester.broadcast_min_feerate().await.unwrap();
        tracing::info!("Broadcast minimum fee rate: {:#}", fee_filter);
        tracing::info!("Press CTRL + C to exit.");
    }
}
