#[path = "../../../tests/support/spv_pool.rs"]
mod fixture;
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;
use clap::Parser;
use std::{fs, path::PathBuf};
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    pool::{OperationKind, PoolClaim, nullifier_digest, output_digest},
};
use zutti_spv_prototype::{
    Result, bounded,
    settlement::{Dependencies, run},
    write_new,
};
use zutti_wallet::spv_pool::{SpvAction, SpvPoolClaim, SpvPoolWitness};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    program: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    prove_deposit: bool,
}
fn main() -> Result<()> {
    let args = Args::parse();
    fs::create_dir(&args.out)?;
    let program = bounded(&args.program, 32 * 1024 * 1024)?;
    let funding = fixture::reservation(support::digest(52), support::digest(51));
    let output = fixture::output(
        funding.claim.pool.domain,
        &fixture::keys(20),
        300_000_000,
        140,
    );
    let deposit = SpvPoolWitness {
        policy: funding.policy,
        claim: SpvPoolClaim {
            policy: funding.claim.policy,
            binding_program: funding.claim.binding_program,
            expiry: 0,
            pool: PoolClaim {
                kind: OperationKind::Deposit,
                domain: funding.claim.pool.domain,
                root: Digest::default(),
                checkpoint: Digest::default(),
                order_tag: Digest::default(),
                order: Digest::default(),
                reservation: Digest::default(),
                value: 300_000_000,
                owner: [4; 20],
                nullifiers: nullifier_digest(&[])?,
                outputs: output_digest(&[output.binding()?])?,
            },
        },
        inputs: vec![],
        outputs: vec![output],
        action: SpvAction::Deposit,
    };
    run(
        &program,
        &deposit,
        Dependencies::Genuine(&[]),
        &args.out.join("deposit"),
        args.prove_deposit,
    )?;
    if !args.prove_deposit {
        for (name, witness) in [
            ("reservation", &funding),
            ("paid", &fixture::terminal(&funding, true)),
            ("unpaid", &fixture::terminal(&funding, false)),
        ] {
            run(
                &program,
                witness,
                Dependencies::Unresolved,
                &args.out.join(name),
                false,
            )?;
        }
        for (name, mode) in [
            ("missing-binding", Dependencies::Missing(0)),
            ("missing-source", Dependencies::Missing(1)),
            ("wrong-binding-image", Dependencies::WrongImage),
            ("wrong-source-journal", Dependencies::WrongJournal),
        ] {
            run(
                &program,
                &fixture::terminal(&funding, true),
                mode,
                &args.out.join(name),
                false,
            )?;
        }
    }
    let mut cases = serde_json::Map::new();
    for (name, witness) in [
        ("reservation", &funding),
        ("paid", &fixture::terminal(&funding, true)),
        ("unpaid", &fixture::terminal(&funding, false)),
    ] {
        let claim = &witness.claim.pool;
        cases.insert(name.into(),serde_json::json!({"journal":hex::encode(witness.claim.to_bytes()?),"journal_digest":witness.claim.journal_digest()?.to_string(),"kind":claim.kind as u8,"domain":claim.domain.to_string(),"policy":witness.claim.policy.to_string(),"binding_image":witness.claim.binding_program.to_string(),"expiry":witness.claim.expiry,"root":claim.root.to_string(),"checkpoint":claim.checkpoint.to_string(),"order_tag":claim.order_tag.to_string(),"order":claim.order.to_string(),"reservation":claim.reservation.to_string(),"value":claim.value,"owner":hex::encode(claim.owner),"nullifiers":claim.nullifiers.to_string(),"outputs":claim.outputs.to_string()}));
    }
    write_new(
        &args.out.join("journals.json"),
        &serde_json::to_vec_pretty(&cases)?,
    )?;
    write_new(
        &args.out.join("scope.json"),
        &serde_json::to_vec_pretty(
            &serde_json::json!({"inputs":"Generated public Ironwood effects, synthetic unverified header context and source observations","source_program":"placeholder","binding_program":"placeholder","proofs_for_conditional_operations":false,"source_anchor_authenticated":false,"end_to_end_swap":false,"settlement_authority":false,"paid_jobs":0}),
        )?,
    )?;
    Ok(())
}
