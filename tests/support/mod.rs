#![allow(dead_code, clippy::unwrap_used)]
use zutti_protocol::{
    Digest, Secret32,
    amount::{Amount, Asset, FeeBps},
    domain::Domain,
    note::FixedOutput,
    order::{Direction, OrderInput, OrderTerms},
    policy::PaymentWindow,
};

pub fn digest(byte: u8) -> Digest {
    Digest([byte; 32])
}
pub fn secret(byte: u8) -> Secret32 {
    Secret32::from_bytes([byte; 32]).unwrap()
}
pub fn domain() -> Domain {
    Domain::new(
        digest(1),
        11155111,
        [2; 20],
        [3; 20],
        digest(4),
        digest(5),
        digest(6),
    )
    .unwrap()
}
pub fn input() -> OrderInput {
    let usdc = Amount::parse(Asset::Usdc, "199").unwrap();
    OrderInput {
        domain: domain(),
        direction: Direction::ZecToUsdc,
        offer: digest(7),
        zec: Amount::parse(Asset::Zec, "5").unwrap(),
        usdc,
        receiver: [8; 43],
        payout: FixedOutput::new(digest(9), digest(10), usdc).unwrap(),
        refund: FixedOutput::new(digest(11), digest(12), usdc).unwrap(),
        gross_output: Amount::parse(Asset::Usdc, "200").unwrap(),
        fee_bps: FeeBps::new(50).unwrap(),
        fee_caps: [
            Amount::parse(Asset::Zec, "0.001").unwrap(),
            Amount::parse(Asset::Eth, "0.005").unwrap(),
            Amount::parse(Asset::Usdc, "1").unwrap(),
        ],
        window: PaymentWindow::new(10, 11, digest(13), 20).unwrap(),
        nonce: digest(14),
    }
}
pub fn terms() -> OrderTerms {
    OrderTerms::new(input()).unwrap()
}
