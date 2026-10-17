use zutti_protocol::{
    Digest,
    amount::{Amount, Asset},
    commitment::hash,
    domain::Domain,
    encoding::Canonical,
    escrow::ReservationOpening,
};
#[path = "../../../tests/support/mod.rs"]
mod support;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let domain = Domain::new(
        Digest([1; 32]),
        11155111,
        [2; 20],
        [3; 20],
        Digest([4; 32]),
        Digest([5; 32]),
        Digest([6; 32]),
    )?;
    let amount = Amount::parse(Asset::Zec, "5")?;
    let terms = support::terms();
    let order_randomness = support::secret(21);
    let order = terms.commitment(&order_randomness)?;
    let reservation = ReservationOpening::new(order, Digest([24; 32]), support::secret(25))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "suite": "zutti-sha256-development-v1",
            "domain_bytes": hex::encode(domain.to_bytes()?), "domain_id": domain.id()?.to_string(),
            "amount": {"asset_id": 1, "decimal": "5", "units": "500000000", "bytes": hex::encode(amount.to_bytes()?)},
            "framing": {"tag": "vector", "parts_hex": ["61", "6263"], "digest": hash(b"vector", &[b"a", b"bc"]).to_string()},
            "order": {"bytes":hex::encode(terms.to_bytes()?),"randomness":hex::encode(order_randomness.expose()),"commitment":order.to_string(),"use_tag":terms.use_tag()?.to_string(),"nonce":hex::encode([14;32])},
            "reservation": {"transaction_id":hex::encode([24;32]),"randomness":hex::encode([25;32]),"commitment":reservation.commitment(&terms,&order_randomness)?.to_string()},
            "escrow": {"secret":hex::encode([26;32]),"terminal_nullifier":ReservationOpening::terminal_nullifier(domain.id()?,&support::secret(26)).to_string()}
        }))?
    );
    Ok(())
}
