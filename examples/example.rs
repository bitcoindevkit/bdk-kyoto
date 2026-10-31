use bdk_kyoto::builder::{Builder, BuilderExt};
use bdk_kyoto::{HashCheckpoint, Info, Receiver, SyncConfig, Warning};
use bdk_wallet::bitcoin::Network;
use bdk_wallet::{KeychainKind, Wallet};
use tokio::select;

const RECV: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/0/*)";
const CHANGE: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/1/*)";
const NETWORK: Network = Network::Signet;

/* Sync a bdk wallet */

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

    let mut wallet = Wallet::create(RECV, CHANGE)
        .network(NETWORK)
        .create_wallet_no_persist()?;

    let checkpoint = HashCheckpoint::from_genesis(NETWORK);
    let start_height = checkpoint.height;
    let sync_config = SyncConfig::wallet_recovery_sync(checkpoint).build();

    // The light client builder handles the logic of inserting the SPKs
    let client = Builder::new(NETWORK)
        .build_with_wallet(&wallet, sync_config)
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
    let update = update_subscriber.update().await?;
    wallet.apply_update(update)?;
    tracing::info!("Tx count: {}", wallet.transactions().count());
    tracing::info!("Balance: {:#}", wallet.balance().total());
    let last_revealed = wallet.derivation_index(KeychainKind::External);
    tracing::info!("Last revealed External: {:?}", last_revealed);
    tracing::info!(
        "Last revealed Internal: {:?}",
        wallet.derivation_index(KeychainKind::Internal)
    );
    tracing::info!("Local chain tip: {}", wallet.local_chain().tip().height());
    let next = wallet.reveal_next_address(KeychainKind::External).address;
    tracing::info!("Next receiving address: {next}");
    let fee_filter = requester.broadcast_min_feerate().await.unwrap();
    tracing::info!("Broadcast minimum fee rate: {:#}", fee_filter);
    return Ok(());
}

fn percent_complete(start: u32, height: u32, tip: u32) -> f32 {
    let total = tip.saturating_sub(start).max(1);
    let done = height.saturating_sub(start).min(total);
    done as f32 / total as f32 * 100.0
}
