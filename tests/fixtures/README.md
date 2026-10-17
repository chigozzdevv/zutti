# Protocol fixtures

`encoding.json` is a fixed cross-language vector for the development SHA-256 transcript and canonical integer/domain encoding. Rust and Node verify it independently. Recreate candidate output with `cargo run --locked -p zutti-protocol --example vectors`; review changes rather than silently regenerating expected values during a test.

The block text files contain unmodified public Zcash block vectors from the published `zebra-test 4.0.0` crate, including both network genesis blocks and three consecutive post-NU5 blocks. `provenance.json` records upstream paths, revision and SHA-256 hashes. They are data fixtures, never trusted bootstrap checkpoints. Preserve the accompanying upstream license files.

Other deterministic identities, amounts and synthetic notes in `tests/support` and wallet tests are exclusively test inputs. They are not network deployment parameters, real wallet keys or consensus proofs.
