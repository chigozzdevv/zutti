#![allow(clippy::unwrap_used)]
use zutti_protocol::{
    Digest, Error,
    amount::{Amount, Asset, FeeBps},
    domain::Domain,
    encoding::Canonical,
    policy::PaymentWindow,
};

#[test]
fn exact_decimal_round_trips() {
    for (asset, value, units) in [
        (Asset::Zec, "5", 500_000_000),
        (Asset::Usdc, "199", 199_000_000),
        (Asset::Zec, "2.4875", 248_750_000),
        (Asset::Eth, "0.000000000000000001", 1),
    ] {
        let amount = Amount::parse(asset, value).unwrap();
        assert_eq!(amount.units(), units);
        assert_eq!(amount.to_string(), value);
        assert_eq!(
            Amount::from_bytes(&amount.to_bytes().unwrap()).unwrap(),
            amount
        );
    }
}
#[test]
fn invalid_decimal_inputs_never_round_silently() {
    for text in [
        "",
        " ",
        " 1",
        "+1",
        "-1",
        "01",
        ".5",
        "1.",
        "1e2",
        "NaN",
        "1..2",
        "１",
        "1.0000001",
    ] {
        assert!(Amount::parse(Asset::Usdc, text).is_err(), "{text:?}");
    }
    assert!(Amount::parse(Asset::Zec, "21000000.00000001").is_err());
    assert!(Amount::parse(Asset::Eth, "99999999999999999999999999999999999999999").is_err());
}
#[test]
fn fee_rounding_is_once_on_gross_output_and_overflow_safe() {
    let fee = FeeBps::new(50).unwrap();
    assert_eq!(
        Amount::parse(Asset::Usdc, "200")
            .unwrap()
            .net_after_fee(fee)
            .to_string(),
        "199"
    );
    assert_eq!(
        Amount::parse(Asset::Zec, "2.5")
            .unwrap()
            .net_after_fee(fee)
            .to_string(),
        "2.4875"
    );
    for bps in [0, 1, 50, 9999] {
        let gross = Amount::from_units(Asset::Usdc, u128::MAX).unwrap();
        let net = gross.net_after_fee(FeeBps::new(bps).unwrap());
        assert!(net.units() <= gross.units());
        assert_eq!(
            net.checked_add(gross.checked_sub(net).unwrap()).unwrap(),
            gross
        );
    }
    assert!(FeeBps::new(10_000).is_err());
}
#[test]
fn canonical_decoder_rejects_every_truncation_trailing_bytes_and_bad_tags() {
    let value = Amount::parse(Asset::Usdc, "5.123456").unwrap();
    let encoded = value.to_bytes().unwrap();
    for length in 0..encoded.len() {
        assert!(Amount::from_bytes(&encoded[..length]).is_err());
    }
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert_eq!(Amount::from_bytes(&trailing), Err(Error::Trailing));
    let mut bad = encoded;
    bad[0] = 255;
    assert!(Amount::from_bytes(&bad).is_err());
}
#[test]
fn domain_roundtrip_and_unknown_version_rejection() {
    let domain = Domain::new(
        Digest([1; 32]),
        11155111,
        [2; 20],
        [3; 20],
        Digest([4; 32]),
        Digest([5; 32]),
        Digest([6; 32]),
    )
    .unwrap();
    let bytes = domain.to_bytes().unwrap();
    assert_eq!(Domain::from_bytes(&bytes).unwrap(), domain);
    let mut changed = bytes;
    changed[8] = 2;
    assert_eq!(Domain::from_bytes(&changed), Err(Error::Version));
    assert!(
        Domain::new(
            Digest([0; 32]),
            1,
            [2; 20],
            [3; 20],
            Digest([4; 32]),
            Digest([5; 32]),
            Digest([6; 32])
        )
        .is_err()
    );
}
#[test]
fn expiry_is_a_height_with_an_inclusive_last_valid_block() {
    let window = PaymentWindow::new(10, 11, Digest([1; 32]), 20).unwrap();
    assert!(window.require_remaining(19, 1).is_ok());
    assert!(window.require_remaining(20, 1).is_err());
    assert!(window.require_remaining(u32::MAX, 1).is_err());
    assert!(PaymentWindow::new(10, 10, Digest([1; 32]), 20).is_err());
    assert!(PaymentWindow::new(10, 11, Digest([1; 32]), 0).is_err());
}
