# bdk-kyoto

A chain backend for BDK using compact filter based light client [kyoto](https://github.com/rustaceanrob/kyoto).

### Issues
- Is the witness stripped from txs fetched over p2p?
    - https://bitcoin.stackexchange.com/questions/116952/bitcoin-p2p-network-unable-to-receive-the-full-block-data-witness-stripped-off

### TODO
- Make sure balance is correct after sync
- Add `new_request` to Client struct - to create a new sync request without dropping the client
- Test tx broadcast
- Add example-cli
- Consider moving to a branch in bdk repo
