use bdk_kyoto::builder::{NodeBuilder, NodeBuilderExt};
use bdk_kyoto::{LightClient, RequesterExt, ScanType};
use bdk_wallet::bitcoin::Network;
use bdk_wallet::{KeychainKind, Wallet};
use tokio::select;

/// Peer address whitelist
const RECV: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/0/*)";
const CHANGE: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/1/*)";

/* Sync a bdk wallet */

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = tracing_subscriber::FmtSubscriber::new();
    tracing::subscriber::set_global_default(subscriber)?;

    let mut wallet = Wallet::create(RECV, CHANGE)
        .network(Network::Signet)
        .lookahead(30)
        .create_wallet_no_persist()?;

    // With no persistence, each scan type is a recovery.
    let scan_type = ScanType::Recovery {
        from_height: 170_000,
    };

    // The light client builder handles the logic of inserting the SPKs
    let LightClient {
        requester,
        mut log_subscriber,
        mut info_subscriber,
        mut warning_subscriber,
        mut update_subscriber,
        node,
    } = NodeBuilder::new(Network::Signet)
        .build_with_wallet(&wallet, scan_type)
        .unwrap();

    tokio::task::spawn(async move { node.run().await });

    // Sync and apply updates. We can do this in a continual loop while the "application" is running.
    // Often this would occur on a separate thread than the underlying application user interface.
    loop {
        select! {
            update = update_subscriber.update() => {
                wallet.apply_update(update?)?;
                tracing::info!("Tx count: {}", wallet.transactions().count());
                tracing::info!("Balance: {}", wallet.balance().total().to_sat());
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
                tracing::info!(
                    "Broadcast minimum fee rate: {:#}",
                    fee_filter
                );
                requester.add_revealed_scripts(&wallet)?;
            },
            log = log_subscriber.recv() => {
                if let Some(log) = log {
                    tracing::info!("{log}")
                }
            }
            info = info_subscriber.recv() => {
                if let Some(info) = info {
                    tracing::info!("{info}")
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
