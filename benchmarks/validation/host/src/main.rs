use clap::{Parser, ValueEnum};
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::Instant};
use zcash_protocol::consensus::{MAIN_NETWORK, TEST_NETWORK};
use zutti_protocol::encoding::Writer;
use zutti_source::{authorization, block::ParsedBlock, trees::CandidateTrees};

use zutti_validation_runner::{Result, bounded, program};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Network {
    Mainnet,
    Testnet,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Pool {
    Sprout,
    Sapling,
    Orchard,
    Ironwood,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Tamper {
    Binding,
    Proof,
}

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    block: PathBuf,
    #[arg(long, value_enum)]
    network: Network,
    #[arg(long, value_enum)]
    pool: Pool,
    #[arg(long)]
    actions: Option<usize>,
    #[arg(long)]
    transaction: Option<usize>,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=16))]
    repetitions: u16,
    #[arg(long, default_value_t = 1 << 35, value_parser = clap::value_parser!(u64).range(1..=1 << 38))]
    cycle_limit: u64,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    program: Option<PathBuf>,
    #[arg(long)]
    inspect: bool,
    #[arg(long, value_enum, conflicts_with = "inspect")]
    tamper: Option<Tamper>,
    #[arg(long, conflicts_with_all = ["tamper", "inspect"])]
    trees: bool,
    #[arg(long, conflicts_with_all = ["tamper", "inspect", "trees"])]
    context: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.out.try_exists()? {
        return Err("profile output already exists".into());
    }
    let raw = bounded(&args.block, 2_000_000)?;
    let block = match args.network {
        Network::Mainnet => ParsedBlock::parse(&raw, &MAIN_NETWORK),
        Network::Testnet => ParsedBlock::parse(&raw, &TEST_NETWORK),
    }?;
    let (index, tx) = block
        .transactions()
        .iter()
        .enumerate()
        .find(|(index, tx)| {
            if args.transaction.is_some_and(|expected| *index != expected)
                || tx
                    .transparent_bundle()
                    .is_some_and(|b| !b.is_coinbase() && !b.vin.is_empty())
            {
                return false;
            }
            let pools = [
                tx.sprout_bundle().is_some(),
                tx.sapling_bundle().is_some(),
                tx.orchard_bundle().is_some(),
                tx.ironwood_bundle().is_some(),
            ];
            let (selected, count) = match args.pool {
                Pool::Sprout => (0, tx.sprout_bundle().map_or(0, |b| b.joinsplits.len())),
                Pool::Sapling => (
                    1,
                    tx.sapling_bundle().map_or(0, |b| {
                        b.shielded_spends().len() + b.shielded_outputs().len()
                    }),
                ),
                Pool::Orchard => (2, tx.orchard_bundle().map_or(0, |b| b.actions().len())),
                Pool::Ironwood => (3, tx.ironwood_bundle().map_or(0, |b| b.actions().len())),
            };
            pools[selected]
                && pools.iter().filter(|present| **present).count() == 1
                && args.actions.is_none_or(|expected| count == expected)
        })
        .ok_or("no matching isolated public authorization workload in this block")?;
    let coinbase = tx
        .transparent_bundle()
        .is_some_and(|bundle| bundle.is_coinbase());
    if args.context {
        zutti_source::transaction::check_context(
            tx,
            block.branch(),
            block.claimed_height(),
            block.header().time,
            coinbase,
        )?;
    } else if !args.trees {
        authorization::verify(tx, &[])?;
    }
    if args.inspect {
        println!(
            "{}",
            serde_json::json!({"height":block.claimed_height(),"transaction_index":index,
                "txid":tx.txid().to_string(),"pool":format!("{:?}",args.pool),
                "orchard_actions":tx.orchard_bundle().map_or(0,|b|b.actions().len()),
                "ironwood_actions":tx.ironwood_bundle().map_or(0,|b|b.actions().len()),
                "circuit_version":tx.orchard_bundle().or_else(||tx.ironwood_bundle()).map(|b|format!("{:?}",b.bundle_version().circuit_version())),
                "native_authorization_verified":true})
        );
        return Ok(());
    }
    let mut input_raw = raw.clone();
    if let Some(tamper) = args.tamper {
        let bundle = tx
            .orchard_bundle()
            .or_else(|| tx.ironwood_bundle())
            .ok_or("tampering requires an Orchard or Ironwood bundle")?;
        let target = match tamper {
            Tamper::Binding => {
                let signature: [u8; 64] = bundle.authorization().binding_signature().into();
                signature.to_vec()
            }
            Tamper::Proof => bundle.authorization().proof().as_ref().to_vec(),
        };
        let positions: Vec<_> = raw
            .windows(target.len())
            .enumerate()
            .filter_map(|(offset, bytes)| (bytes == target).then_some(offset))
            .collect();
        if positions.len() != 1 {
            return Err("authorization mutation must have one exact location".into());
        }
        input_raw[positions[0] + target.len() / 2] ^= 1;
        let changed = match args.network {
            Network::Mainnet => ParsedBlock::parse(&input_raw, &MAIN_NETWORK),
            Network::Testnet => ParsedBlock::parse(&input_raw, &TEST_NETWORK),
        }?;
        if authorization::verify(&changed.transactions()[index], &[])
            != Err(authorization::Error::Orchard)
        {
            return Err("native verification did not reject the changed authorization".into());
        }
    }
    let mut writer = Writer::default();
    writer.fixed(if args.context {
        b"ZUTTICX1"
    } else {
        b"ZUTTIVAL"
    });
    writer.u8(match args.network {
        Network::Mainnet => 0,
        Network::Testnet => 1,
    });
    writer.u32(index.try_into()?);
    writer.u16(args.repetitions);
    writer.bytes(&input_raw)?;
    let input = writer.finish();
    let expected_journal = if args.trees {
        let mut trees = CandidateTrees::default();
        let mut digest = Sha256::new();
        digest.update(Sha256::digest(&input));
        for _ in 0..args.repetitions {
            trees.append(block.commitment_updates())?;
            let roots = trees.roots();
            for root in [
                trees.sprout_root(),
                roots.sapling,
                roots.orchard,
                roots.ironwood,
            ] {
                digest.update(root.0);
            }
        }
        digest.finalize().to_vec()
    } else {
        Sha256::digest(&input).to_vec()
    };
    let binary = if args.trees {
        "zutti-tree-profile"
    } else if args.context {
        "context"
    } else {
        "zutti-validation-profile"
    };
    let program = program(binary, args.program.as_deref())?;
    fs::create_dir_all(&args.out)?;
    fs::write(args.out.join("program.elf"), &program)?;
    let mut stdin = (input.len() as u32).to_le_bytes().to_vec();
    stdin.extend_from_slice(&input);
    fs::write(args.out.join("stdin.bin"), &stdin)?;
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&stdin)
        .session_limit(Some(args.cycle_limit))
        .segment_limit_po2(20);
    let started = Instant::now();
    let result =
        ExternalProver::new("zutti-validation-profile", "r0vm").execute(builder.build()?, &program);
    if let Some(tamper) = args.tamper {
        let error = result.err().ok_or("guest accepted changed authorization")?;
        let error = error.to_string();
        if !error.contains("invalid transaction authorization") {
            return Err(format!("guest failed before authorization rejection: {error}").into());
        }
        let report = serde_json::json!({"schema":1,"network":format!("{:?}",args.network),
            "pool":format!("{:?}",args.pool),"height":block.claimed_height(),"transaction_index":index,
            "txid":tx.txid().to_string(),"tamper":format!("{tamper:?}"),
            "native_rejected":true,"guest_rejected":true,"error":error,
            "image_id":risc0_binfmt::compute_image_id(&program)?.to_string(),
            "original_block_sha256":hex::encode(Sha256::digest(&raw)),
            "changed_block_sha256":hex::encode(Sha256::digest(&input_raw)),
            "program_sha256":hex::encode(Sha256::digest(&program)),
            "stdin_sha256":hex::encode(Sha256::digest(&stdin)),
            "seconds":started.elapsed().as_secs_f64(),
            "proof_generated":false,"source_state_admitted":false,"paid_jobs":0});
        fs::write(
            args.out.join("rejection.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        println!("{report}");
        return Ok(());
    }
    let session = result?;
    if session.exit_code != risc0_zkvm::ExitCode::Halted(0)
        || session.journal.bytes != expected_journal
    {
        return Err("validation profile execution or journal mismatch".into());
    }
    let report = serde_json::json!({"schema":1,"network":format!("{:?}",args.network),
        "pool":format!("{:?}",args.pool),"trees":args.trees,"context":args.context,"coinbase":coinbase,
        "height":block.claimed_height(),"transaction_index":index,
        "txid":tx.txid().to_string(),"repetitions":args.repetitions,"cycles":session.cycles(),
        "segments":session.segments.len(),"padded_cycles":session.segments.iter().map(|s|1_u64 << s.po2).sum::<u64>(),
        "seconds":started.elapsed().as_secs_f64(),"image_id":risc0_binfmt::compute_image_id(&program)?.to_string(),
        "block_sha256":hex::encode(Sha256::digest(&raw)),"program_sha256":hex::encode(Sha256::digest(&program)),
        "stdin_sha256":hex::encode(Sha256::digest(&stdin)),"journal_matches":true,
        "orchard_actions":tx.orchard_bundle().map_or(0,|b|b.actions().len()),
        "ironwood_actions":tx.ironwood_bundle().map_or(0,|b|b.actions().len()),
        "sapling_spends":tx.sapling_bundle().map_or(0,|b|b.shielded_spends().len()),
        "sapling_outputs":tx.sapling_bundle().map_or(0,|b|b.shielded_outputs().len()),
        "tree_updates_per_repetition":{"sprout":block.commitment_updates().sprout().len(),
            "sapling":block.commitment_updates().sapling().len(),
            "orchard":block.commitment_updates().orchard().len(),
            "ironwood":block.commitment_updates().ironwood().len()},
        "circuit_version":tx.orchard_bundle().or_else(||tx.ironwood_bundle()).map(|b|format!("{:?}",b.bundle_version().circuit_version())),
        "proof_generated":false,"source_state_admitted":false,"paid_jobs":0});
    fs::write(
        args.out.join("execution.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
