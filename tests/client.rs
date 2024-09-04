// #![allow(unused)]
use std::net::IpAddr;
use std::time::Duration;
use tokio::task;
use tokio::time;

use bdk_kyoto::builder::LightClientBuilder;
use bdk_kyoto::logger::PrintLogger;
use bdk_kyoto::TrustedPeer;
use bdk_testenv::bitcoincore_rpc::RpcApi;
use bdk_testenv::bitcoind;
use bdk_testenv::TestEnv;
use bdk_wallet::bitcoin::{Amount, Network};
use bdk_wallet::chain::spk_client::FullScanResult;
use bdk_wallet::CreateParams;
use bdk_wallet::KeychainKind;

const EXTERNAL_DESCRIPTOR: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
const INTERNAL_DESCRIPTOR: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";

fn testenv() -> anyhow::Result<TestEnv> {
    use bdk_testenv::Config;
    let mut conf = bitcoind::Conf::default();
    conf.p2p = bitcoind::P2P::Yes;
    conf.args.push("-blockfilterindex=1");
    conf.args.push("-peerblockfilters=1");

    TestEnv::new_with_config(Config {
        bitcoind: conf,
        ..Default::default()
    })
}

fn wait_for_height(env: &TestEnv, height: u32) -> anyhow::Result<()> {
    while env.rpc_client().get_block_count()? < height as u64 {
        let _ = time::sleep(Duration::from_millis(256));
    }
    Ok(())
}

fn init_node(
    env: &TestEnv,
    wallet: &bdk_wallet::Wallet,
) -> anyhow::Result<(bdk_kyoto::Node, bdk_kyoto::Client<KeychainKind>)> {
    let peer = env.bitcoind.params.p2p_socket.unwrap();
    let ip: IpAddr = peer.ip().clone().into();
    let port = peer.port();
    let mut peer = TrustedPeer::from_ip(ip);
    peer.port = Some(port);
    let path = tempfile::tempdir()?.path().join("kyoto-data");
    Ok(LightClientBuilder::new(&wallet)
        .peers(vec![peer])
        .data_dir(path)
        .connections(1)
        .build()?)
}

#[tokio::test]
async fn update_returns_blockchain_data() -> anyhow::Result<()> {
    let env = testenv()?;

    let miner = env
        .rpc_client()
        .get_new_address(None, None)?
        .assume_checked();

    let wallet = CreateParams::new(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
        .network(Network::Regtest)
        .create_wallet_no_persist()?;

    let index = 2;
    let addr = wallet.peek_address(KeychainKind::External, index).address;

    // build node/client
    let (node, mut client) = init_node(&env, &wallet)?;

    // mine blocks
    let _hashes = env.mine_blocks(100, Some(miner.clone()))?;
    wait_for_height(&env, 101)?;

    // send tx
    let amt = Amount::from_btc(0.21)?;
    let txid = env.send(&addr, amt)?;
    let hashes = env.mine_blocks(1, Some(miner))?;
    wait_for_height(&env, 102)?;

    // run node
    task::spawn(async move { node.run().await });
    let logger = PrintLogger::new();
    // get update
    if let Some(update) = client.update(&logger).await {
        let FullScanResult {
            tx_update,
            chain_update,
            last_active_indices,
        } = update;
        // graph tx and anchor
        let tx = tx_update.txs.iter().next().unwrap();
        let (anchor, anchor_txid) = tx_update.anchors.first().unwrap().clone();
        assert_eq!(anchor_txid, txid);
        assert_eq!(anchor.block_id.height, 102);
        assert_eq!(anchor.block_id.hash, hashes[0]);
        let txout = tx.output.iter().find(|txout| txout.value == amt).unwrap();
        assert_eq!(txout.script_pubkey, addr.script_pubkey());
        // chain
        let update_cp = chain_update.unwrap();
        assert_eq!(update_cp.height(), 102);
        // keychain
        assert_eq!(
            last_active_indices,
            [(KeychainKind::External, index)].into()
        );
    }

    client.shutdown().await?;

    Ok(())
}

#[tokio::test]
async fn test_reorg_is_handled() -> anyhow::Result<()> {
    let env = testenv()?;

    let miner = env
        .rpc_client()
        .get_new_address(None, None)?
        .assume_checked();

    let mut wallet = CreateParams::new(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
        .network(Network::Regtest)
        .create_wallet_no_persist()?;

    let index = 2;
    let addr = wallet.peek_address(KeychainKind::External, index).address;

    // build node/client
    let (node, mut client) = init_node(&env, &wallet)?;

    // mine blocks
    let _hashes = env.mine_blocks(100, Some(miner.clone()))?;
    wait_for_height(&env, 101)?;

    task::spawn(async move { node.run().await });

    let amt = Amount::from_btc(0.21)?;

    let logger = PrintLogger::new();
    for i in 1..=2 {
        let _ = env.send(&addr, amt)?;
        let _ = env.mine_blocks(1, Some(miner.clone()))?;

        wait_for_height(&env, 102)?;

        // reorg
        if i % 2 == 0 {
            _ = env.reorg(3)?;
        }

        let update = client.update(&logger).await;
        if let Some(update) = update {
            wallet.apply_update(update).unwrap();
        }

        if i % 2 == 0 {
            assert_eq!(wallet.balance().total(), Amount::from_sat(0));
        } else {
            assert_eq!(wallet.balance().total(), amt);
        }
    }

    // FIXME: Error receiving channel has been closed
    client.shutdown().await?;

    Ok(())
}
