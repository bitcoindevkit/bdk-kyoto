use bdk_kyoto::builder::LightClientBuilder;
use bdk_kyoto::{Event, LightClient, LogLevel};
use bdk_wallet::bitcoin::Network;
use bdk_wallet::Wallet;

/* Sync a bdk wallet using events*/

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
    let change_desc = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";

    let mut wallet = Wallet::create(desc, change_desc)
        .network(Network::Signet)
        .lookahead(30)
        .create_wallet_no_persist()?;

    // The light client builder handles the logic of inserting the SPKs
    let LightClient {
        sender: _,
        mut receiver,
        node,
    } = LightClientBuilder::new(&wallet)
        .scan_after(170_000)
        .build()
        .unwrap();

    tokio::task::spawn(async move { node.run().await });

    loop {
        if let Some(event) = receiver.next_event(LogLevel::Info).await {
            match event {
                Event::Log(log) => println!("INFO: {log}"),
                Event::Warning(warning) => println!("WARNING: {warning}"),
                Event::ScanResponse(full_scan_result) => {
                    wallet.apply_update(full_scan_result).unwrap();
                    println!(
                        "INFO: Balance in BTC: {}",
                        wallet.balance().total().to_btc()
                    );
                }
                Event::PeersFound => println!("INFO: Connected to all necessary peers."),
                Event::TxSent(txid) => println!("INFO: Broadcast transaction: {txid}"),
                Event::TxFailed(failure_payload) => {
                    println!("WARNING: Transaction failed to broadcast: {failure_payload:?}")
                }
                Event::StateChange(node_state) => println!("NEW TASK: {node_state}"),
                Event::BlocksDisconnected(_) => {}
            }
        }
    }
}
