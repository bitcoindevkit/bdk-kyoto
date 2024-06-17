# bdk-kyoto

### Issues
- Balance appears incorrect after sync
- Is the witness stripped from txs fetched over p2p?
    - https://bitcoin.stackexchange.com/questions/116952/bitcoin-p2p-network-unable-to-receive-the-full-block-data-witness-stripped-off

### TODO
- Add `new_request` to Client struct - to create a new sync request without dropping the client
- Test tx broadcast
- Add example-cli
- Consider moving to a branch in bdk repo
