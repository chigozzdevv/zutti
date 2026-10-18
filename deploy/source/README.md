# Testnet block source

Run these commands from the repository root:

```sh
mkdir -p .data/zebra-state .data/zebra-network .data/zebra-rpc
cargo install zebrad --version 7.0.0-rc.0 --locked --no-default-features \
  --root .data/zebra --target-dir .data/zebra-build --jobs 2
.data/zebra/bin/zebrad -c deploy/source/zebra.testnet.toml start
```

This pins the NU7-capable Zebra release. Its RPC and P2P listeners bind to loopback. Zebra generates the owner-only RPC cookie at `.data/zebra-rpc/.cookie`; the source runner reads it directly. The configuration contains no wallet or proving credentials.

Zebra supplies complete serialized blocks. Zutti's proof program validates its own prefix from the exact network genesis. A synchronized Zebra database and Zebra's checkpoint validation do not constitute a Zutti source receipt. The current source program implements NU7 rules; a current-chain receipt still requires proving the complete prefix and passing source admission.

The local node can sync while the proof runner processes older batches. Keep the state, retained program ELFs, source database and pending proof jobs across restarts. Measure disk usage as both databases grow; the node cache alone does not describe the complete proving footprint.
