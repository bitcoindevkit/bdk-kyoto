// #![allow(unused)]
use bdk_kyoto::{state::Idle, wallets::Single, SyncPolicyType};
use bdk_wallet::chain::{DescriptorExt, DescriptorId};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;
use tokio::time;

use bdk_kyoto::builder::{Builder, BuilderExt};
use bdk_kyoto::{HashCheckpoint, LightClient, SyncConfig, TrustedPeer};
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
) -> anyhow::Result<LightClient<Idle, Single>> {
    let peer = env.bitcoind.params.p2p_socket.unwrap();
    let ip: IpAddr = (*peer.ip()).into();
    let port = peer.port();
    let peer: TrustedPeer = (ip, Some(port)).into();
    Ok(Builder::new(Network::Regtest)
        .add_peer(peer)
        .required_peers(1)
        .build_with_wallet(wallet, SyncConfig::sync_from_last_checkpoint().build())?)
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
    let client = init_node(&env, &wallet)?;
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

    let client = init_node(&env, &wallet)?;
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

    let client = init_node(&env, &wallet)?;
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

    let client = init_node(&env, &wallet)?;
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
async fn update_is_cancel_safe() -> anyhow::Result<()> {
    let env = testenv()?;

    let mut wallet = CreateParams::new(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
        .network(Network::Regtest)
        .create_wallet_no_persist()?;
    let addr = wallet.peek_address(KeychainKind::External, 0).address;

    let client = init_node(&env, &wallet)?;
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

    // Race `update()` against a very short timeout so the future is repeatedly
    // dropped mid-flight until an attempt fits inside the deadline.
    let mut cancellations = 0;
    let res = loop {
        match tokio::time::timeout(Duration::from_millis(2), update_subscriber.update()).await {
            Ok(res) => break res?,
            Err(_) => {
                cancellations += 1;
                if cancellations % 20 == 0 {
                    let _ = env.mine_blocks(1, Some(miner.clone()))?;
                }
            }
        }
    };
    assert!(cancellations > 0);

    let anchor = res
        .tx_update
        .anchors
        .iter()
        .find(|(_, id)| *id == txid)
        .map(|(anchor, _)| *anchor)
        .expect("tx must be anchored");
    assert_eq!(anchor.block_id.height, 102);
    assert_eq!(anchor.block_id.hash, blockhash);
    let update_tip = res.chain.as_ref().unwrap().clone();
    wallet.apply_update(res).unwrap();

    // Balance reflects the confirmed receive.
    assert_eq!(wallet.balance().total(), amt);

    // The wallet's chain tip agrees with the update and with bitcoind.
    let wallet_tip = wallet.local_chain().tip();
    assert!(wallet_tip.height() >= 102);
    assert_eq!(wallet_tip.height(), update_tip.height());
    assert_eq!(wallet_tip.hash(), update_tip.hash());
    let rpc_hash_at_tip = env
        .rpc_client()
        .get_block_hash(wallet_tip.height() as u64)?;
    assert_eq!(wallet_tip.hash(), rpc_hash_at_tip);

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

    let client = init_node(&env, &wallet)?;
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
    let peer: TrustedPeer = (ip, Some(port)).into();
    let client = Builder::new(Network::Regtest)
        .add_peer(peer)
        .required_peers(1)
        .build_with_wallets(vec![
            (&wallet, SyncConfig::sync_from_last_checkpoint().build()),
            (&wallet_two, SyncConfig::sync_from_last_checkpoint().build()),
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

fn init_node_with_config<P: SyncPolicyType>(
    env: &TestEnv,
    wallet: &bdk_wallet::Wallet,
    sync_config: SyncConfig<P>,
) -> anyhow::Result<LightClient<Idle, Single>> {
    let peer = env.bitcoind.params.p2p_socket.unwrap();
    let ip: IpAddr = (*peer.ip()).into();
    let peer: TrustedPeer = (ip, Some(peer.port())).into();
    Ok(Builder::new(Network::Regtest)
        .add_peer(peer)
        .required_peers(1)
        .build_with_wallet(wallet, sync_config)?)
}

// Pay `index` on the external keychain of a wallet with `wallet_lookahead`, sync with
// `sync_config` and check the payment is found.
async fn assert_finds_payment_to_index<P: SyncPolicyType>(
    env: &TestEnv,
    wallet_lookahead: u32,
    index: u32,
    sync_config: impl FnOnce(&TestEnv) -> anyhow::Result<SyncConfig<P>>,
) -> anyhow::Result<()> {
    let miner = env
        .rpc_client()
        .get_new_address(None, None)?
        .assume_checked();
    let mut wallet = CreateParams::new(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
        .network(Network::Regtest)
        .lookahead(wallet_lookahead)
        .create_wallet_no_persist()?;

    env.mine_blocks(100, Some(miner.clone()))?;
    let addr = wallet.peek_address(KeychainKind::External, index).address;
    let amt = Amount::from_btc(0.21)?;
    env.send(&addr, amt)?;
    env.mine_blocks(1, Some(miner))?;
    wait_for_height(env, 102).await?;

    let client = init_node_with_config(env, &wallet, sync_config(env)?)?;
    let (client, _, mut update_subscriber) = client.subscribe();
    let client = client.start();

    let update = update_subscriber.update().await?;
    assert_eq!(
        update.last_active_indices,
        [(KeychainKind::External, index)].into()
    );
    wallet.apply_update(update)?;
    assert_eq!(wallet.balance().total(), amt);

    client.requester().shutdown()?;
    Ok(())
}

#[tokio::test]
async fn sync_checks_scripts_within_wallet_lookahead() -> anyhow::Result<()> {
    let env = testenv()?;
    // Index 100 is beyond the default lookahead, but within this wallet's.
    assert_finds_payment_to_index(&env, 200, 100, |_| {
        Ok(SyncConfig::sync_from_last_checkpoint().build())
    })
    .await
}

// The amount paid to `index`, distinct per index so a balance shows which payments were found.
fn amount_for_index(index: u32) -> Amount {
    Amount::from_sat(1_000_000 + index as u64)
}

// Send one transaction paying `indices` on the external keychain, with outputs in that order.
fn send_ordered(env: &TestEnv, wallet: &bdk_wallet::Wallet, indices: &[u32]) -> anyhow::Result<()> {
    use bdk_testenv::bitcoincore_rpc::jsonrpc::serde_json::{json, Map, Value};
    let outputs = indices
        .iter()
        .map(|&index| {
            let addr = wallet.peek_address(KeychainKind::External, index).address;
            let mut output = Map::new();
            output.insert(addr.to_string(), json!(amount_for_index(index).to_btc()));
            Value::Object(output)
        })
        .collect::<Vec<Value>>();
    let rpc = env.rpc_client();
    let raw: String = rpc.call("createrawtransaction", &[json!([]), json!(outputs)])?;
    // Keep the change after our outputs so their order is preserved.
    let funded: Value = rpc.call(
        "fundrawtransaction",
        &[json!(raw), json!({ "changePosition": indices.len() })],
    )?;
    let signed: Value = rpc.call("signrawtransactionwithwallet", &[funded["hex"].clone()])?;
    let _: Value = rpc.call("sendrawtransaction", &[signed["hex"].clone()])?;
    Ok(())
}

// Mine one block per entry of `blocks`, each with one transaction paying those external indices
// in order, then recover a wallet with `wallet_lookahead` from genesis. Returns the last used
// indices and balance of the recovered wallet.
async fn recover_after_payments(
    wallet_lookahead: u32,
    blocks: &[&[u32]],
) -> anyhow::Result<(BTreeMap<KeychainKind, u32>, Amount)> {
    let env = testenv()?;
    let miner = env
        .rpc_client()
        .get_new_address(None, None)?
        .assume_checked();
    let mut wallet = CreateParams::new(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
        .network(Network::Regtest)
        .lookahead(wallet_lookahead)
        .create_wallet_no_persist()?;

    env.mine_blocks(100, Some(miner.clone()))?;
    for indices in blocks {
        send_ordered(&env, &wallet, indices)?;
        env.mine_blocks(1, Some(miner.clone()))?;
    }
    wait_for_height(&env, 101 + blocks.len() as u32).await?;

    let genesis = env.rpc_client().get_block_hash(0)?;
    let sync_config = SyncConfig::wallet_recovery_sync(HashCheckpoint::new(0, genesis)).build();
    let client = init_node_with_config(&env, &wallet, sync_config)?;
    let (client, _, mut update_subscriber) = client.subscribe();
    let client = client.start();

    let update = update_subscriber.update().await?;
    let last_active_indices = update.last_active_indices.clone();
    wallet.apply_update(update)?;

    client.requester().shutdown()?;
    Ok((last_active_indices, wallet.balance().total()))
}

#[tokio::test]
async fn recovery_follows_payments_beyond_wallet_lookahead() -> anyhow::Result<()> {
    // Each index is only within the lookahead once the block before it has been applied.
    let (last_active, balance) = recover_after_payments(25, &[&[20], &[40], &[60]]).await?;
    assert_eq!(last_active, [(KeychainKind::External, 60)].into());
    assert_eq!(
        balance,
        amount_for_index(20) + amount_for_index(40) + amount_for_index(60)
    );
    Ok(())
}

#[tokio::test]
async fn recovery_finds_payments_revealed_later_in_same_block() -> anyhow::Result<()> {
    // Index 40 comes first in the transaction, but is only within the lookahead once the output
    // to index 20 is found.
    let (last_active, balance) = recover_after_payments(25, &[&[40, 20]]).await?;
    assert_eq!(last_active, [(KeychainKind::External, 40)].into());
    assert_eq!(balance, amount_for_index(20) + amount_for_index(40));
    Ok(())
}

#[tokio::test]
async fn recovery_gap_limit_is_the_wallet_lookahead() -> anyhow::Result<()> {
    // Index 30 is paid before index 10 reveals it, so it is beyond the lookahead when its block
    // is checked, and that block is never downloaded.
    let (last_active, balance) = recover_after_payments(25, &[&[30], &[10]]).await?;
    assert_eq!(last_active, [(KeychainKind::External, 10)].into());
    assert_eq!(balance, amount_for_index(10));

    // A larger lookahead covers it.
    let (last_active, balance) = recover_after_payments(50, &[&[30], &[10]]).await?;
    assert_eq!(last_active, [(KeychainKind::External, 30)].into());
    assert_eq!(balance, amount_for_index(10) + amount_for_index(30));
    Ok(())
}
