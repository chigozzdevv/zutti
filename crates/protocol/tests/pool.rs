#![allow(clippy::unwrap_used)]

use sha2::{Digest as _, Sha256};
use zutti_protocol::{
    Digest,
    domain::Domain,
    encoding::Canonical,
    pool::{
        OperationKind, PoolClaim, PoolPath, PoolTree, SourceUpdateClaim, nullifier_digest,
        output_digest,
    },
};

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/pool.json")).unwrap()
}
fn digest(value: &serde_json::Value, name: &str) -> Digest {
    Digest::from_hex(value[name].as_str().unwrap()).unwrap()
}
fn domain() -> Digest {
    Domain::new(
        Digest([17; 32]),
        31337,
        [51; 20],
        [68; 20],
        Digest([102; 32]),
        Digest([85; 32]),
        Digest([119; 32]),
    )
    .unwrap()
    .id()
    .unwrap()
}

#[test]
fn public_pool_and_source_journals_match_independent_vectors() {
    let v = vectors();
    assert_eq!(domain(), digest(&v, "domain"));
    let claim = PoolClaim {
        kind: OperationKind::Transfer,
        domain: domain(),
        root: Digest([34; 32]),
        checkpoint: Digest::default(),
        order_tag: Digest::default(),
        order: Digest::default(),
        reservation: Digest::default(),
        value: 0,
        owner: [0; 20],
        nullifiers: nullifier_digest(&[Digest([136; 32])]).unwrap(),
        outputs: output_digest(&[(
            Digest([153; 32]),
            Digest(Sha256::digest([0xaa, 0xbb]).into()),
        )])
        .unwrap(),
    };
    assert_eq!(
        claim.to_bytes().unwrap(),
        hex::decode(v["pool_journal"].as_str().unwrap()).unwrap()
    );
    assert_eq!(claim.journal_digest().unwrap(), digest(&v, "pool_digest"));
    assert_eq!(
        PoolClaim::from_bytes(&claim.to_bytes().unwrap()).unwrap(),
        claim
    );
    let mut work = [0; 32];
    work[31] = 42;
    let source = SourceUpdateClaim {
        genesis: Digest([17; 32]),
        policy: Digest([85; 32]),
        previous: Digest::default(),
        previous_stable: Digest::default(),
        state: Digest([170; 32]),
        checkpoint: Digest([187; 32]),
        work_be: work,
    };
    assert_eq!(
        source.to_bytes().unwrap(),
        hex::decode(v["source_journal"].as_str().unwrap()).unwrap()
    );
    assert_eq!(
        Digest(Sha256::digest(source.to_bytes().unwrap()).into()),
        digest(&v, "source_digest")
    );
    assert_eq!(
        SourceUpdateClaim::from_bytes(&source.to_bytes().unwrap()).unwrap(),
        source
    );
}

#[test]
fn pool_roots_match_vectors_and_membership_binds_every_field() {
    let v = vectors();
    let mut tree = PoolTree::new(domain()).unwrap();
    assert_eq!(tree.root(), digest(&v, "empty_root"));
    for (index, byte) in [153, 170].into_iter().enumerate() {
        let commitment = Digest([byte; 32]);
        assert_eq!(tree.append(commitment).unwrap(), index as u32);
        assert_eq!(
            tree.root(),
            Digest::from_hex(v["roots"][index].as_str().unwrap()).unwrap()
        );
        let path = tree.prove(index as u32).unwrap();
        path.verify(domain(), commitment, tree.root()).unwrap();
        assert_eq!(
            PoolPath::from_bytes(&path.to_bytes().unwrap()).unwrap(),
            path
        );
        assert!(
            path.verify(Digest([1; 32]), commitment, tree.root())
                .is_err()
        );
        assert!(path.verify(domain(), Digest([2; 32]), tree.root()).is_err());
        let mut changed = path.clone();
        changed.position ^= 1;
        assert!(changed.verify(domain(), commitment, tree.root()).is_err());
        changed = path;
        changed.siblings[0].0[0] ^= 1;
        assert!(changed.verify(domain(), commitment, tree.root()).is_err());
    }
    let root = tree.root();
    assert!(tree.append(Digest([153; 32])).is_err());
    assert_eq!(tree.root(), root);
    assert!(tree.prove(2).is_err());
}

#[test]
fn operation_encodings_reject_malformed_modes_counts_and_replay_material() {
    assert!(nullifier_digest(&[Digest([1; 32]); 2]).is_err());
    assert!(nullifier_digest(&[Digest([1; 32]); 9]).is_err());
    assert!(output_digest(&[(Digest::default(), Digest([1; 32]))]).is_err());
    let bytes = hex::decode(vectors()["pool_journal"].as_str().unwrap()).unwrap();
    for length in [0, 10, bytes.len() - 1] {
        assert!(PoolClaim::from_bytes(&bytes[..length]).is_err());
    }
    let mut changed = bytes.clone();
    changed.push(0);
    assert!(PoolClaim::from_bytes(&changed).is_err());
    changed = bytes.clone();
    changed[10] = 1;
    assert!(PoolClaim::from_bytes(&changed).is_err());
    changed = bytes;
    changed[10] = 5;
    assert!(PoolClaim::from_bytes(&changed).is_err());
}
