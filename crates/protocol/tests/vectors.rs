#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/mod.rs"]
mod support;
use zutti_protocol::{
    commitment::hash, encoding::Canonical, escrow::ReservationOpening, order::OrderTerms,
    policy::PaymentWindow,
};

#[test]
fn fixed_vector_matches_published_bytes() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/encoding.json")).unwrap();
    assert_eq!(
        hex::encode(support::domain().to_bytes().unwrap()),
        fixture["domain_bytes"].as_str().unwrap()
    );
    assert_eq!(
        support::domain().id().unwrap().to_string(),
        fixture["domain_id"].as_str().unwrap()
    );
    assert_eq!(
        hash(b"vector", &[b"a", b"bc"]).to_string(),
        fixture["framing"]["digest"].as_str().unwrap()
    );
    assert_eq!(
        support::terms()
            .commitment(&support::secret(21))
            .unwrap()
            .to_string(),
        fixture["order"]["commitment"].as_str().unwrap()
    );
}
#[test]
fn order_roundtrip_rejects_truncation_and_trailing_data() {
    let terms = support::terms();
    let encoded = terms.to_bytes().unwrap();
    assert_eq!(OrderTerms::from_bytes(&encoded).unwrap(), terms);
    for len in 0..encoded.len() {
        assert!(OrderTerms::from_bytes(&encoded[..len]).is_err());
    }
    let mut changed = encoded;
    changed.push(0);
    assert!(OrderTerms::from_bytes(&changed).is_err());
}
#[test]
fn private_commitments_bind_terms_randomness_and_transaction() {
    let terms = support::terms();
    let randomness = support::secret(21);
    let order = terms.commitment(&randomness).unwrap();
    assert_ne!(order, terms.commitment(&support::secret(22)).unwrap());
    let mut input = support::input();
    input.receiver = [23; 43];
    assert_ne!(
        order,
        OrderTerms::new(input)
            .unwrap()
            .commitment(&randomness)
            .unwrap()
    );
    let mut input = support::input();
    input.window = PaymentWindow::new(10, 11, support::digest(13), 21).unwrap();
    assert_ne!(
        order,
        OrderTerms::new(input)
            .unwrap()
            .commitment(&randomness)
            .unwrap()
    );
    let first = ReservationOpening::new(order, support::digest(24), support::secret(25)).unwrap();
    let second = ReservationOpening::new(order, support::digest(26), support::secret(25)).unwrap();
    assert_ne!(
        first.commitment(&terms, &randomness).unwrap(),
        second.commitment(&terms, &randomness).unwrap()
    );
    assert!(first.commitment(&terms, &support::secret(27)).is_err());
    assert_ne!(terms.use_tag().unwrap(), support::domain().id().unwrap());
    assert_ne!(hash(b"a", &[b"b", b"cd"]), hash(b"a", &[b"bc", b"d"]));
    assert_ne!(hash(b"a", &[b"bc"]), hash(b"ab", &[b"c"]));
}
#[test]
fn memo_has_no_transaction_id_dependency() {
    let terms = support::terms();
    let order = terms.commitment(&support::secret(21)).unwrap();
    let memo = terms.memo(order).unwrap();
    assert_eq!(memo[0], 0xff);
    assert_eq!(&memo[43..75], &order.0);
    assert_eq!(&memo[75..107], &terms.window().anchor_hash().0);
    assert!(memo[107..].iter().all(|b| *b == 0));
}
#[test]
fn no_output_or_fee_substitution() {
    let mut input = support::input();
    input.refund = input.payout;
    assert!(OrderTerms::new(input).is_err());
    let mut input = support::input();
    input.gross_output = input.usdc;
    assert!(OrderTerms::new(input).is_err());
    assert_eq!(format!("{:?}", support::secret(99)), "Secret32([REDACTED])");
}
