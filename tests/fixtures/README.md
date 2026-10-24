# Protocol fixtures

`encoding.json` is a fixed cross-language vector for the development SHA-256 transcript and canonical integer/domain encoding. Rust and Node verify it independently. Recreate candidate output with `cargo run --locked -p zutti-protocol --example vectors`; review changes rather than silently regenerating expected values during a test.

The block text files contain unmodified public Zcash block vectors from the published `zebra-test 4.0.0` crate, including both network genesis blocks and three consecutive post-NU5 blocks. `provenance.json` records upstream paths, revision and SHA-256 hashes. They are data fixtures, never trusted bootstrap checkpoints. Preserve the accompanying upstream license files.

Other deterministic identities, amounts and synthetic notes in `tests/support` and wallet tests are exclusively test inputs. They are not network deployment parameters, real wallet keys or consensus proofs.

`sapling-outputs.json` retains the genuine public output proofs used by the source authorization regression tests. It is test input, not a source checkpoint or settlement receipt.

`source-genesis-proof.json` contains generated public Groth16 evidence for the exact mainnet genesis under source image `c5a1d868e176647966ce6e728860c9225c99e05a0d8880a474e1aeeb652ec669`. It contains no swap witness or key. Boundless fulfilled request `0x68f4482fd6d407aba396080bc8bcf1d3957d7483a2ec5c9a` in Base transaction `0xdc5124f3a42780992cce2509d32ac0324fee7c8cf29153fb2dda76b56a6bc0d2`. The fixture proves genesis processing, not a current Zcash checkpoint or a swap.

`source-prefix-proof.json` proves consecutive mainnet blocks 1 through 4 while verifying the genuine genesis predecessor inside the guest. `source-update-proof.json` verifies that prefix receipt and authenticates stable checkpoint 1 under the demonstration's three-block policy. Both contain generated public proof evidence only. They are historical-prefix fixtures, not live-network bootstrap endorsements.

`pool-lifecycle.json` contains public deployment metadata, output ciphertexts and genuine Groth16 receipts for a deterministic local-chain deposit/transfer/withdrawal cycle. Its note keys were generated for public demonstration, not real users. The fixture is shared by native receipt-verification tests and the actual Ethereum pool-execution test. It does not contain a mined Zcash payment.

The `source-testnet-genesis-proof.json` fixture is a genuine Boundless Groth16 receipt for the exact public Testnet genesis under the retained source V2 program and its 100-block source policy. It was fulfilled on Base in [transaction 0x804596…60b24](https://basescan.org/tx/0x804596276ec438281fecc34ebdf33857a7135cdbb84994cc52dd6b598f460b24). Submission-to-fulfillment was 52 seconds and the proving price was 19,976,269,603,949 wei. The prefix contains only genesis; it is not a current-chain proof or an executed swap.

`source-optimized-genesis-proof.json` contains public Testnet genesis evidence under source V6 and its three-block policy. The source program supports batches of at most 128 blocks; older programs retain their original limits. The optimized genesis receipt was fulfilled in [transaction 0x6ad850…db229](https://basescan.org/tx/0x6ad8500d74ebbde4ba60a4a86480fc1eaf70e6ff51dd95a8cee8c9e6a74db229) after 53 seconds at a proving fee of 26,635,026,138,599 wei, excluding transaction gas. Native tests independently verify its seal, compare its claim with native genesis validation and reject altered evidence.

`source-optimized-range-128-proof.json` proves public Testnet blocks 1–128 as a range starting at the optimized genesis state. `source-optimized-prefix-128-proof.json` verifies that range and the genuine genesis receipt inside the source guest, producing the full prefix through height 128. Fulfillment transactions are [range](https://basescan.org/tx/0x6b9c51de96aac35c732a44db6ea66b93437d90c04f0f8acd97c3d721a603c351) and [prefix](https://basescan.org/tx/0x43b3904cdd3ebd4b6da02fb17f62e62970b685e93cc3c2013947116ab26852ba). The benchmark records their exact times and fees. These fixtures contain no private swap inputs, wallet keys or current-chain admission.
