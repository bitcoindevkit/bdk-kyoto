use bdk_kyoto::builder::{Builder, BuilderExt};
use bdk_kyoto::{Info, Receiver, SyncConfig, Warning};
use bdk_wallet::bitcoin::Network;
use bdk_wallet::chain::DescriptorExt;
use bdk_wallet::{KeychainKind, Wallet};
use tokio::select;

const RECV_ONE: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/0/*)";
const CHANGE_ONE: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/1/*)";
const RECV_TWO: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
const CHANGE_TWO: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
const NETWORK: Network = Network::Signet;

/* Sync multiple BDK wallets */

async fn traces(
    start_height: u32,
    mut info_subscriber: Receiver<Info>,
    mut warning_subscriber: Receiver<Warning>,
) {
    let mut chain_height = start_height;
    loop {
        select! {
            info = info_subscriber.recv() => {
                match info {
                    Some(Info::NewChainHeight(height)) => {
                        chain_height = height;
                        tracing::info!("Header height: {height}");
                    }
                    Some(Info::CFHeadersAdded(height)) => {
                        let percent = percent_complete(start_height, height, chain_height);
                        tracing::info!("Filter headers: {percent:.2}% ({height}/{chain_height})");
                    }
                    Some(Info::FilterAdded(height)) if height % 2_000 == 0 => {
                        let percent = percent_complete(start_height, height, chain_height);
                        tracing::info!("Filters: {percent:.2}% ({height}/{chain_height})");
                    }
                    Some(Info::FilterAdded(_)) => (),
                    Some(info) => tracing::info!("{info}"),
                    None => (),
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

    let start_height = wallet_one
        .local_chain()
        .tip()
        .height()
        .min(wallet_two.local_chain().tip().height());

    // Now build a client that will sync both wallets simultaneously
    let client = Builder::new(NETWORK)
        .wallets()
        .add(&wallet_one, SyncConfig::sync_from_last_checkpoint().build())
        .add(&wallet_two, SyncConfig::new_wallet_sync().build())
        .build()
        .unwrap();
    let (client, logging, mut update_subscriber) = client.subscribe();
    tokio::task::spawn(async move {
        traces(
            start_height,
            logging.info_subscriber,
            logging.warning_subscriber,
        )
        .await
    });
    let client = client.start();
    let requester = client.requester();

    // Sync and apply updates. We can do this in a continual loop while the "application" is running.
    // Often this would occur on a separate thread than the underlying application user interface.
    loop {
        // Updates are grouped with the `DescriptorId` of the public, external descriptor.
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

fn percent_complete(start: u32, height: u32, tip: u32) -> f32 {
    let total = tip.saturating_sub(start).max(1);
    let done = height.saturating_sub(start).min(total);
    done as f32 / total as f32 * 100.0
}
