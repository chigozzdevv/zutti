#[path = "../../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../../tests/support/mod.rs"]
mod support;

use clap::Parser;
use std::{fs, path::PathBuf};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    Digest,
    amount::{Amount, Asset},
    domain::Domain,
    order::OrderTerms,
    policy::PaymentWindow,
};
use zutti_source::spv::Policy;
use zutti_spv_prototype::{Result, binding::run, bounded, write_new};
use zutti_wallet::{
    binding::{PaymentBindingWitness, payment_profile},
    preparation::PreparationWitness,
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    program: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    prove: bool,
}
fn main() -> Result<()> {
    let args = Args::parse();
    fs::create_dir(&args.out)?;
    let program = bounded(&args.program, 32 * 1024 * 1024)?;
    let policy = Policy::new(Digest([51; 32]), NetworkType::Test, 2, 128, 4_465_200)?;
    let fixture = payment::fixture_custom(false, "0.001", 4_465_101, |input| {
        input.domain = Domain::new(
            input.domain.source_genesis(),
            11155111,
            [2; 20],
            [3; 20],
            payment_profile(),
            policy.id().expect("public policy"),
            support::digest(6),
        )
        .expect("public domain");
    });
    let raw = fixture
        .pczt
        .serialize()
        .map_err(|_| "public PCZT encoding failed")?;
    for case in [
        "valid",
        "wrong-recipient",
        "wrong-amount",
        "wrong-expiry",
        "wrong-anchor",
        "wrong-height",
    ] {
        if args.prove && case != "valid" {
            continue;
        }
        let mut input = support::input();
        input.domain = fixture.terms.domain().clone();
        input.window = fixture.terms.window();
        input.receiver = *fixture.terms.receiver();
        match case {
            "wrong-recipient" => input.receiver = [9; 43],
            "wrong-amount" => input.zec = Amount::parse(Asset::Zec, "4")?,
            "wrong-expiry" => {
                input.window = PaymentWindow::new(0, 4_465_100, support::digest(13), 4_465_120)?
            }
            "wrong-anchor" => {
                input.window = PaymentWindow::new(0, 4_465_100, support::digest(15), 4_465_121)?
            }
            "wrong-height" => {
                input.window = PaymentWindow::new(0, 4_465_099, support::digest(13), 4_465_121)?
            }
            _ => {}
        }
        let preparation = PreparationWitness::new(
            OrderTerms::new(input)?,
            support::secret(21),
            support::secret(32),
            raw.clone(),
        )?;
        let witness = PaymentBindingWitness::new(policy, preparation)?;
        run(
            &program,
            &witness,
            &args.out.join(case),
            args.prove,
            case != "valid",
        )?;
    }
    write_new(
        &args.out.join("scope.json"),
        &serde_json::to_vec_pretty(&serde_json::json!({
        "inputs":"Generated public Ironwood PCZT, test-only keys and synthetic order terms",
        "source_policy_program":"Test-only placeholder; not an authenticated source prefix",
        "source_payment_broadcast":false,"paid_jobs":0,"end_to_end_swap":false,"settlement_authority":false,
        "receipt_reuse":false,"checks":"The binding guest directly reevaluates the existing preparation checks"}))?,
    )?;
    Ok(())
}
