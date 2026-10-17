# Consensus verifying keys

These public verification parameters are embedded in the native transaction verifier. They are not checkpoints or spending keys.

Sapling keys are the exact verifying-key prefixes of the official parameter files. Both complete files were checked against the BLAKE2b-512 digests pinned by `zcash_proofs` 0.30.0 before extraction. The Sprout Groth16 key comes from `zebra-consensus` 16.0.0. The historical PHGR key is normalized from the [published Sprout ceremony key](https://github.com/zcash/mpc#assets), whose SHA-256 is `4bd498dae0aacfd8e98dc306338d017d9c08dd0918ead18172bd0aec2fc5df82`.

`provenance.json` records input and output hashes. `scripts/extract-consensus-keys.py` reproduces the files from local copies of the authenticated inputs. It rejects mismatched hashes before writing. It converts PHGR's libsnark Montgomery coordinates to canonical affine coordinates and preserves the nine-input query order.

The Rust verifier checks curve membership and subgroup membership. Historical PHGR tests verify real block transactions and reject changed public inputs and proof points. Sapling and Orchard tests verify real transactions and reject altered binding signatures. Individual proof verification avoids introducing a predictable batch-verification challenge.

Reference implementations: [Zcash PHGR point encoding](https://github.com/zcash/zcash/blob/v2.0.7/src/zcash/Proof.cpp), [PHGR verification equations](https://github.com/zcash/zcash/blob/v2.0.7/src/snark/libsnark/zk_proof_systems/ppzksnark/r1cs_ppzksnark/r1cs_ppzksnark.tcc), [Sapling verifier](https://docs.rs/sapling-crypto/0.7.0/src/sapling_crypto/verifier.rs.html). Reference source licenses are MIT for Zcash/libsnark and MIT/Apache-2.0 for Zebra/librustzcash.
