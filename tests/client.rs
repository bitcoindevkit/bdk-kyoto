// #![allow(unused)]
use bdk_kyoto::{Idle, Single};
use bdk_wallet::chain::{DescriptorExt, DescriptorId};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time;

use bdk_kyoto::builder::{Builder, BuilderExt};
use bdk_kyoto::{LightClient, ScanType, TrustedPeer};
use bdk_testenv::bitcoincore_rpc::RpcApi;
use bdk_testenv::bitcoind;
use bdk_testenv::TestEnv;
use bdk_wallet::bitcoin::{Amount, Network};
use bdk_wallet::CreateParams;
use bdk_wallet::KeychainKind;
use bdk_wallet::Update;

const EXTERNAL_DESCRIPTOR: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/0/*)";
const INTERNAL_DESCRIPTOR: &str = "tr([7d94197e/86'/1'/0']tpubDCyQVJj8KzjiQsFjmb3KwECVXPvMwvAxxZGCP9XmWSopmjW3bCV3wD7TgxrUhiGSueDS1MU5X1Vb1YjYcp8jitXc5fXfdC1z68hDDEyKRNr/1/*)";
const RECV_TWO: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/0/*)";
const CHANGE_TWO: &str = "wpkh([9122d9e0/84'/1'/0']tpubDCYVtmaSaDzTxcgvoP5AHZNbZKZzrvoNH9KARep88vESc6MxRqAp4LmePc2eeGX6XUxBcdhAmkthWTDqygPz2wLAyHWisD299Lkdrj5egY6/1/*)";

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

async fn wait_for_height(env: &TestEnv, height: u32) -> anyhow::Result<()> {
    while env.rpc_client().get_block_count()? < height as u64 {
        time::sleep(Duration::from_millis(256)).await;
    }
    Ok(())
}

fn init_node(
    env: &TestEnv,
    wallet: &bdk_wallet::Wallet,
    tempdir: PathBuf,
) -> anyhow::Result<LightClient<Idle, Single>> {
    let peer = env.bitcoind.params.p2p_socket.unwrap();
    let ip: IpAddr = (*peer.ip()).into();
    let port = peer.port();
    let mut peer = TrustedPeer::from_ip(ip);
    peer.port = Some(port);
    Ok(Builder::new(Network::Regtest)
        .add_peer(peer)
        .data_dir(tempdir)
        .required_peers(1)
        .build_with_wallet(wallet, ScanType::Sync)?)
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
    let tempdir = tempfile::tempdir()?.path().join("kyoto-data");
    let client = init_node(&env, &wallet, tempdir)?;
    let (client, _, mut update_subscriber) = client.subscribe();

    // mine blocks
    let _hashes = env.mine_blocks(100, Some(miner.clone()))?;
    wait_for_height(&env, 101).await?;

    // send tx
    let amt = Amount::from_btc(0.21)?;
    let txid = env.send(&addr, amt)?;
    let hashes = env.mine_blocks(1, Some(miner))?;
    wait_for_height(&env, 102).await?;

    let client = client.start();
    let requester = client.requester();

    // get update
    let res = update_subscriber.update().await?;
    let Update {
        tx_update,
        chain,
        last_active_indices,
    } = res;
    // graph tx and anchor
    let tx = tx_update.txs.first().unwrap();
    let (anchor, anchor_txid) = *tx_update.anchors.iter().next().unwrap();
    assert_eq!(anchor_txid, txid);
    assert_eq!(anchor.block_id.height, 102);
    assert_eq!(anchor.block_id.hash, hashes[0]);
    let txout = tx.output.iter().find(|txout| txout.value == amt).unwrap();
    assert_eq!(txout.script_pubkey, addr.script_pubkey());
    // chain
    let update_cp = chain.unwrap();
    assert_eq!(update_cp.height(), 102);
    // keychain
    assert_eq!(
        last_active_indices,
        [(KeychainKind::External, index)].into()
    );

    requester.shutdown()?;

    Ok(())
}

#[tokio::test]
async fn update_handles_reorg() -> anyhow::Result<()> {
    let env = testenv()?;

    let mut wallet = CreateParams::new(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
        .network(Network::Regtest)
        .create_wallet_no_persist()?;
    let addr = wallet.peek_address(KeychainKind::External, 0).address;

    let tempdir = tempfile::tempdir()?.path().join("kyoto-data");
    let client = init_node(&env, &wallet, tempdir)?;
    let (client, _, mut update_subscriber) = client.subscribe();

    // mine blocks
    let miner = env
        .rpc_client()
        .get_new_address(None, None)?
        .assume_checked();
    let _hashes = env.mine_blocks(100, Some(miner.clone()))?;
    wait_for_height(&env, 101).await?;

    // send tx
    let amt = Amount::from_btc(0.21)?;
    let txid = env.send(&addr, amt)?;
    let hashes = env.mine_blocks(1, Some(miner.clone()))?;
    let blockhash = hashes[0];
    wait_for_height(&env, 102).await?;

    let client = client.start();
    let requester = client.requester();

    // get update
    let res = update_subscriber.update().await?;
    let (anchor, anchor_txid) = *res.tx_update.anchors.iter().next().unwrap();
    assert_eq!(anchor.block_id.hash, blockhash);
    assert_eq!(anchor_txid, txid);
    wallet.apply_update(res).unwrap();

    // reorg
    let hashes = env.reorg(1)?; // 102
    let new_blockhash = hashes[0];
    _ = env.mine_blocks(2, Some(miner))?; // 103
    wait_for_height(&env, 103).await?;

    // expect tx to confirm at same height but different blockhash
    let res = update_subscriber.update().await?;
    let (anchor, anchor_txid) = *res.tx_update.anchors.iter().next().unwrap();
    assert_eq!(anchor_txid, txid);
    assert_eq!(anchor.block_id.height, 102);
    assert_ne!(anchor.block_id.hash, blockhash);
    assert_eq!(anchor.block_id.hash, new_blockhash);
    wallet.apply_update(res).unwrap();

    requester.shutdown()?;

    Ok(())
}

#[tokio::test]
async fn update_handles_dormant_wallet() -> anyhow::Result<()> {
    let env = testenv()?;

    let mut wallet = CreateParams::new(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
        .network(Network::Regtest)
        .create_wallet_no_persist()?;
    let addr = wallet.peek_address(KeychainKind::External, 0).address;

    let tempdir = tempfile::tempdir()?.path().join("kyoto-data");
    let client = init_node(&env, &wallet, tempdir.clone())?;
    let (client, _, mut update_subscriber) = client.subscribe();
    let client = client.start();
    let requester = client.requester();

    // mine blocks
    let miner = env
        .rpc_client()
        .get_new_address(None, None)?
        .assume_checked();
    let _hashes = env.mine_blocks(100, Some(miner.clone()))?;
    wait_for_height(&env, 101).await?;

    // send tx
    let amt = Amount::from_btc(0.21)?;
    let txid = env.send(&addr, amt)?;
    let hashes = env.mine_blocks(1, Some(miner.clone()))?;
    let blockhash = hashes[0];
    wait_for_height(&env, 102).await?;

    // get update
    let res = update_subscriber.update().await?;
    let (anchor, anchor_txid) = *res.tx_update.anchors.iter().next().unwrap();
    assert_eq!(anchor.block_id.hash, blockhash);
    assert_eq!(anchor_txid, txid);
    wallet.apply_update(res).unwrap();

    // shut down then reorg
    requester.shutdown()?;

    let hashes = env.reorg(1)?; // 102
    let new_blockhash = hashes[0];
    _ = env.mine_blocks(20, Some(miner))?; // 122
    wait_for_height(&env, 122).await?;

    let client = init_node(&env, &wallet, tempdir)?;
    let (client, _, mut update_subscriber) = client.subscribe();
    let client = client.start();
    let requester = client.requester();

    // expect tx to confirm at same height but different blockhash
    let res = update_subscriber.update().await?;
    let (anchor, anchor_txid) = *res.tx_update.anchors.iter().next().unwrap();
    assert_eq!(anchor_txid, txid);
    assert_eq!(anchor.block_id.height, 102);
    assert_ne!(anchor.block_id.hash, blockhash);
    assert_eq!(anchor.block_id.hash, new_blockhash);
    wallet.apply_update(res).unwrap();

    requester.shutdown()?;

    Ok(())
}

#[tokio::test]
async fn two_wallets_can_update() -> anyhow::Result<()> {
    let env = testenv()?;

    let mut wallet = CreateParams::new(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
        .network(Network::Regtest)
        .create_wallet_no_persist()?;
    let addr = wallet.peek_address(KeychainKind::External, 0).address;

    let tempdir = tempfile::tempdir()?.path().join("kyoto-data");
    let client = init_node(&env, &wallet, tempdir.clone())?;
    let (client, _, mut update_subscriber) = client.subscribe();
    let client = client.start();
    let requester = client.requester();

    // mine blocks
    let miner = env
        .rpc_client()
        .get_new_address(None, None)?
        .assume_checked();
    let _hashes = env.mine_blocks(100, Some(miner.clone()))?;
    wait_for_height(&env, 101).await?;

    // send tx
    let amt = Amount::from_btc(0.21)?;
    let txid = env.send(&addr, amt)?;
    let hashes = env.mine_blocks(1, Some(miner.clone()))?;
    let blockhash = hashes[0];
    wait_for_height(&env, 102).await?;

    // get update
    let res = update_subscriber.update().await?;
    let (anchor, anchor_txid) = *res.tx_update.anchors.iter().next().unwrap();
    assert_eq!(anchor.block_id.hash, blockhash);
    assert_eq!(anchor_txid, txid);
    wallet.apply_update(res).unwrap();

    // shut down then reorg
    requester.shutdown()?;

    let hashes = env.reorg(1)?; // 102
    let new_blockhash = hashes[0];
    _ = env.mine_blocks(20, Some(miner))?; // 122
    wait_for_height(&env, 122).await?;

    // add a new wallet to the sync request
    let wallet_two = CreateParams::new(RECV_TWO, CHANGE_TWO)
        .network(Network::Regtest)
        .create_wallet_no_persist()?;
    let peer = env.bitcoind.params.p2p_socket.unwrap();
    let ip: IpAddr = (*peer.ip()).into();
    let port = peer.port();
    let mut peer = TrustedPeer::from_ip(ip);
    peer.port = Some(port);
    let client = Builder::new(Network::Regtest)
        .add_peer(peer)
        .data_dir(tempdir)
        .required_peers(1)
        .build_with_wallets(vec![
            (&wallet, ScanType::Sync),
            (&wallet_two, ScanType::Sync),
        ])?;
    let (client, _, mut update_subscriber) = client.subscribe();
    let client = client.start();
    let requester = client.requester();

    // expect tx to confirm at same height but different blockhash
    let results = update_subscriber.updates().await?;
    let res = results
        .collect::<BTreeMap<DescriptorId, Update>>()
        .get(
            &wallet
                .public_descriptor(KeychainKind::External)
                .descriptor_id(),
        )
        .unwrap()
        .clone();
    let (anchor, anchor_txid) = *res.tx_update.anchors.iter().next().unwrap();
    assert_eq!(anchor_txid, txid);
    assert_eq!(anchor.block_id.height, 102);
    assert_ne!(anchor.block_id.hash, blockhash);
    assert_eq!(anchor.block_id.hash, new_blockhash);
    wallet.apply_update(res).unwrap();

    requester.shutdown()?;

    Ok(())
}
