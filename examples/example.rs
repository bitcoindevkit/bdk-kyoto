use std::net::{IpAddr, Ipv4Addr};

use bdk_kyoto::builder::{NodeBuilder, NodeBuilderExt};
use bdk_kyoto::{LightClient, RequesterExt, ScanType, TrustedPeer};
use bdk_wallet::bitcoin::{p2p::ServiceFlags, Network};
use bdk_wallet::{KeychainKind, Wallet};
use tokio::select;

/// Peer address whitelist
const PEERS: &[IpAddr] = &[IpAddr::V4(Ipv4Addr::new(23, 137, 57, 100))];

/* Sync a bdk wallet */

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
    let change_desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";

    let subscriber = tracing_subscriber::FmtSubscriber::new();
    tracing::subscriber::set_global_default(subscriber)?;

    let peers: Vec<TrustedPeer> = PEERS
        .iter()
        .map(|ip| {
            let mut peer = TrustedPeer::from_ip(*ip);
            peer.set_services(ServiceFlags::P2P_V2);
            peer
        })
        .collect();

    let mut wallet = Wallet::create(desc, change_desc)
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
        .add_peers(peers)
        .build_with_wallet(&wallet, scan_type)
        .unwrap();

    tokio::task::spawn(async move { node.run().await });

    // Sync and apply updates. We can do this in a continual loop while the "application" is running.
    // Often this would occur on a separate thread than the underlying application user interface.
    loop {
        select! {
            update = update_subscriber.update() => {
                if let Some(update) = update {
                    wallet.apply_update(update)?;
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
                }
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
