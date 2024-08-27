use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use bdk_kyoto::builder::LightClientBuilder;
use bdk_kyoto::logger::TraceLogger;
use bdk_wallet::bitcoin::Network;
use bdk_wallet::{KeychainKind, Wallet};
use kyoto::{ServiceFlags, TrustedPeer};

/// Peer address whitelist
const PEERS: &[IpAddr] = &[IpAddr::V4(Ipv4Addr::new(23, 137, 57, 100))];

/* Sync a bdk wallet */

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = tracing_subscriber::FmtSubscriber::new();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
    let change_desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";

    let peers = PEERS
        .into_iter()
        .map(|ip| {
            let mut peer = TrustedPeer::from_ip(*ip);
            peer.set_services(ServiceFlags::P2P_V2);
            peer
        })
        .collect();

    let mut wallet = Wallet::create(desc, change_desc)
        .network(Network::Signet)
        .create_wallet_no_persist()?;

    // The light client builder handles the logic of inserting the SPKs
    let (mut node, mut client) = LightClientBuilder::new(&wallet)
        .scan_after(170_000)
        .peers(peers)
        .logger(Arc::new(TraceLogger::new()))
        .build()
        .unwrap();

    tokio::task::spawn(async move { node.run().await });

    tracing::info!(
        "Balance before sync: {} sats",
        wallet.balance().total().to_sat()
    );

    // Sync and apply updates. We can do this a continual loop while the "application" is running.
    // Often this loop would be on a separate "Task" in a Swift app for instance
    loop {
        if let Some(update) = client.update().await {
            wallet.apply_update(update)?;
            // Do something here to add more scripts?
            tracing::info!("Tx count: {}", wallet.transactions().count());
            tracing::info!("Balance: {}", wallet.balance().total().to_sat());
            let last_revealed = wallet.derivation_index(KeychainKind::External).unwrap();
            tracing::info!(
                "Last revealed External: {}",
                last_revealed
            );
            tracing::info!(
                "Last revealed Internal: {}",
                wallet.derivation_index(KeychainKind::Internal).unwrap()
            );
            tracing::info!("Local chain tip: {}", wallet.local_chain().tip().height());
            let next = wallet.peek_address(KeychainKind::External, last_revealed + 1);
            tracing::info!("Next receiving address: {next}");
        }
    }
}
