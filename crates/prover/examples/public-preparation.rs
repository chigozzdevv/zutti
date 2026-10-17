#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;

use zutti_prover::remote::export_public_preparation;
use zutti_wallet::preparation::PreparationWitness;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("public job directory is required")?;
    let fixture = payment::fixture(false);
    let witness = PreparationWitness::new(
        fixture.terms,
        fixture.randomness,
        support::secret(32),
        fixture
            .pczt
            .serialize()
            .map_err(|_| "fixture encoding failed")?,
    )?;
    let statement = export_public_preparation(&witness, std::path::Path::new(&directory))?;
    println!(
        "{}",
        serde_json::json!({"synthetic_public_witness": true, "image_id": statement.program().to_string(), "settlement_authority": false})
    );
    Ok(())
}
