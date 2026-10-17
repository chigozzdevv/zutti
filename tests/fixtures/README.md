# Protocol fixtures

`encoding.json` is a fixed cross-language vector for the development SHA-256 transcript and canonical integer/domain encoding. Rust and Node verify it independently. Recreate candidate output with `cargo run --locked -p zutti-protocol --example vectors`; review changes rather than silently regenerating expected values during a test.

The block text files contain unmodified public Zcash block vectors from the published `zebra-test 4.0.0` crate, including both network genesis blocks and three consecutive post-NU5 blocks. `provenance.json` records upstream paths, revision and SHA-256 hashes. They are data fixtures, never trusted bootstrap checkpoints. Preserve the accompanying upstream license files.

Other deterministic identities, amounts and synthetic notes in `tests/support` and wallet tests are exclusively test inputs. They are not network deployment parameters, real wallet keys or consensus proofs.

`source-genesis-proof.json` contains generated public Groth16 evidence for the exact mainnet genesis under source image `c5a1d868e176647966ce6e728860c9225c99e05a0d8880a474e1aeeb652ec669`. It contains no swap witness or key. Boundless fulfilled request `0x68f4482fd6d407aba396080bc8bcf1d3957d7483a2ec5c9a` in Base transaction `0xdc5124f3a42780992cce2509d32ac0324fee7c8cf29153fb2dda76b56a6bc0d2`. The fixture proves genesis processing, not a current Zcash checkpoint or a swap.

`source-prefix-proof.json` proves consecutive mainnet blocks 1 through 4 while verifying the genuine genesis predecessor inside the guest. `source-update-proof.json` verifies that prefix receipt and authenticates stable checkpoint 1 under the demonstration's three-block policy. Both contain generated public proof evidence only. They are historical-prefix fixtures, not live-network bootstrap endorsements.

`pool-lifecycle.json` contains public deployment metadata, output ciphertexts and genuine Groth16 receipts for a deterministic local-chain deposit/transfer/withdrawal cycle. Its note keys were generated for public demonstration, not real users. The fixture is shared by native receipt-verification tests and the actual Ethereum pool-execution test. It does not contain a mined Zcash payment.
