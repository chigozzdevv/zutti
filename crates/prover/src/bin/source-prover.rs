#[path = "source-prover/bootstrap.rs"]
mod bootstrap;
#[path = "source-prover/sync.rs"]
mod sync;
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType, TEST_NETWORK};
use zutti_protocol::{encoding::Canonical, pool::SourceUpdateClaim, statement::ProofKind};
use zutti_prover::{
    admission::SourceAdmission,
    artifact::ProofArtifact,
    local::LocalProver,
    pipeline::SourcePipeline,
    source::{MAX_SOURCE_CYCLES, SourceJob, SourcePrograms, source_image_id, update_image_id},
};
use zutti_source::{
    block::ParsedBlock,
    bootstrap::GenesisBlock,
    checkpoint::SourceCheckpoint,
    proof::{SourceInput, SourceReceiptClaim, SourceWitness},
    range::{SourceRangeClaim, SourceRangeWitness},
    state::SourceCandidate,
    transition::StateSnapshot,
    update::{PreviousUpdate, SourceUpdateWitness},
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Network {
    Mainnet,
    Testnet,
}
impl Network {
    fn kind(self) -> NetworkType {
        match self {
            Self::Mainnet => NetworkType::Main,
            Self::Testnet => NetworkType::Test,
        }
    }
}
#[derive(Debug, clap::Args)]
struct Profile {
    #[arg(long, value_enum)]
    network: Network,
    #[arg(long, default_value_t = 0)]
    index_start: u32,
    #[arg(long, default_value_t = 100)]
    confirmations: u32,
    #[arg(long)]
    programs: Option<PathBuf>,
}
impl Profile {
    fn programs(&self) -> Result<SourcePrograms, Box<dyn std::error::Error>> {
        match &self.programs {
            Some(path) => SourcePrograms::read(path),
            None => Ok(SourcePrograms::current()),
        }
    }
    fn policy(&self, programs: &SourcePrograms) -> zutti_source::proof::SourcePolicy {
        programs.policy(self.network.kind(), self.index_start, self.confirmations)
    }
}
#[derive(Debug, clap::Args)]
struct Chain {
    #[command(flatten)]
    profile: Profile,
    #[arg(long)]
    genesis: PathBuf,
}
impl Chain {
    fn programs(&self) -> Result<SourcePrograms, Box<dyn std::error::Error>> {
        self.profile.programs()
    }
    fn policy(&self, programs: &SourcePrograms) -> zutti_source::proof::SourcePolicy {
        self.profile.policy(programs)
    }
    fn source(
        &self,
        prefix: &[PathBuf],
        programs: &SourcePrograms,
    ) -> Result<SourceCandidate, Box<dyn std::error::Error>> {
        let genesis =
            GenesisBlock::parse(&block_bytes(&self.genesis)?, self.profile.network.kind())?;
        let mut source = SourceCandidate::from_genesis(
            &genesis,
            self.policy(programs).id()?,
            self.profile.index_start,
        )?;
        for path in prefix {
            let bytes = block_bytes(path)?;
            let block = match self.profile.network {
                Network::Mainnet => ParsedBlock::parse(&bytes, &MAIN_NETWORK)?,
                Network::Testnet => ParsedBlock::parse(&bytes, &TEST_NETWORK)?,
            };
            source.append(&block)?;
        }
        Ok(source)
    }
}
#[derive(Debug, Parser)]
#[command(name = "source-prover")]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Subcommand)]
enum Command {
    Status,
    Sync(sync::Options),
    Bootstrap(bootstrap::Options),
    Genesis {
        #[command(flatten)]
        chain: Chain,
        #[arg(long)]
        out: PathBuf,
    },
    Continue {
        #[command(flatten)]
        chain: Chain,
        #[arg(long)]
        previous: PathBuf,
        #[arg(long)]
        prefix: Vec<PathBuf>,
        #[arg(long, required = true)]
        block: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    Range {
        #[command(flatten)]
        chain: Chain,
        #[arg(long)]
        prefix: Vec<PathBuf>,
        #[arg(long, required = true)]
        block: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    AggregateRanges {
        #[command(flatten)]
        chain: Profile,
        #[arg(long, required = true)]
        proof: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    FoldRange {
        #[command(flatten)]
        chain: Profile,
        #[arg(long)]
        prefix: PathBuf,
        #[arg(long)]
        range: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    Update {
        #[command(flatten)]
        chain: Chain,
        #[arg(long, required = true)]
        block: Vec<PathBuf>,
        #[arg(long)]
        proof: PathBuf,
        #[arg(long, requires = "previous_checkpoint")]
        previous: Option<PathBuf>,
        #[arg(long, requires = "previous")]
        previous_checkpoint: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    InitStore {
        #[command(flatten)]
        chain: Chain,
        #[arg(long)]
        database: PathBuf,
    },
    AdvanceStore {
        #[command(flatten)]
        chain: Profile,
        #[arg(long)]
        database: PathBuf,
        #[arg(long, required = true)]
        block: Vec<PathBuf>,
    },
    StoreStatus {
        #[command(flatten)]
        chain: Profile,
        #[arg(long)]
        database: PathBuf,
    },
    ExportStore {
        #[command(flatten)]
        chain: Profile,
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    AcceptStore {
        #[command(flatten)]
        chain: Profile,
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        proof: PathBuf,
    },
    ImportStore {
        #[command(flatten)]
        chain: Profile,
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        result: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    Execute {
        #[arg(long)]
        job: PathBuf,
        #[arg(long, default_value = "r0vm")]
        prover: PathBuf,
    },
    Prove {
        #[arg(long)]
        job: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "r0vm")]
        prover: PathBuf,
    },
    Verify {
        #[arg(long)]
        proof: PathBuf,
    },
    Admit {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        proof: PathBuf,
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        checkpoint: PathBuf,
    },
}
fn block_bytes(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(4_000_101)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4_000_100 {
        return Err("block input exceeds its bound".into());
    }
    if bytes
        .iter()
        .all(|b| b.is_ascii_whitespace() || b.is_ascii_hexdigit())
    {
        Ok(hex::decode(
            bytes
                .iter()
                .copied()
                .filter(|b| !b.is_ascii_whitespace())
                .collect::<Vec<_>>(),
        )?)
    } else {
        Ok(bytes)
    }
}
fn read_claim(artifact: &ProofArtifact) -> Result<SourceReceiptClaim, Box<dyn std::error::Error>> {
    if artifact.statement().kind() != ProofKind::SourceTransition {
        return Err("expected a source transition proof".into());
    }
    artifact.verify(artifact.statement())?;
    Ok(SourceReceiptClaim::from_bytes(
        artifact.statement().payload(),
    )?)
}
fn print_store(pipeline: &SourcePipeline) -> Result<(), Box<dyn std::error::Error>> {
    let store = pipeline.store();
    let state = store.candidate().state_snapshot()?;
    let proved_height = store
        .evidence()
        .map(ProofArtifact::from_bytes)
        .transpose()?
        .map(|artifact| {
            SourceReceiptClaim::from_bytes(artifact.statement().payload()).map(|claim| claim.height)
        })
        .transpose()?;
    println!(
        "{}",
        serde_json::json!({"height": state.height()?, "proved_height":proved_height, "tip": state.tip()?.to_string(), "state": state.commitment()?.to_string(), "pending": store.pending().is_some(), "current_prefix_proved": store.pending().is_none() && store.evidence().is_some(), "current_chain_admitted": false})
    );
    Ok(())
}
fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        Command::Sync(options) => sync::run(options)?,
        Command::Bootstrap(options) => bootstrap::run(options)?,
        Command::Status => println!(
            "{}",
            serde_json::json!({"source_image":source_image_id().to_string(),"update_image":update_image_id().to_string(),"recursive_receipts":"groth16","pool_settlement":"unavailable","durable_source":"implemented","supported_through":"NU7","testnet_nu7":"implemented","current_chain_bootstrap":"unavailable"})
        ),
        Command::Genesis { chain, out } => {
            let programs = chain.programs()?;
            let witness = SourceWitness {
                policy: chain.policy(&programs),
                input: SourceInput::Genesis(block_bytes(&chain.genesis)?),
            };
            SourceJob::transition_with(&witness, None, &programs)?
                .export_public(&out, 268_435_456)?;
            println!(
                "{}",
                serde_json::json!({"job":out,"kind":"genesis","proof_generated":false})
            );
        }
        Command::Range {
            chain,
            prefix,
            block,
            out,
        } => {
            let programs = chain.programs()?;
            let mut source = chain.source(&prefix, &programs)?;
            let transition = source.record_transition(
                block
                    .iter()
                    .map(|p| block_bytes(p))
                    .collect::<Result<Vec<_>, _>>()?,
            )?;
            let witness = SourceRangeWitness::transition(chain.policy(&programs), transition)?;
            SourceJob::range(&witness, &[], &programs)?.export_public(&out, MAX_SOURCE_CYCLES)?;
            println!(
                "{}",
                serde_json::json!({"job":out,"kind":"source_range","proof_generated":false,"settlement_authority":false})
            );
        }
        Command::AggregateRanges { chain, proof, out } => {
            let programs = chain.programs()?;
            let proofs = proof
                .iter()
                .map(|p| ProofArtifact::read(p))
                .collect::<Result<Vec<_>, _>>()?;
            let ranges = proofs
                .iter()
                .map(|p| {
                    if p.statement().kind() != ProofKind::SourceRange {
                        return Err(zutti_protocol::Error::Invalid("source range proof kind"));
                    }
                    SourceRangeClaim::from_bytes(p.statement().payload())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let witness = SourceRangeWitness::Aggregate {
                policy: chain.policy(&programs),
                ranges,
            };
            SourceJob::range(&witness, &proofs.iter().collect::<Vec<_>>(), &programs)?
                .export_public(&out, MAX_SOURCE_CYCLES)?;
            println!(
                "{}",
                serde_json::json!({"job":out,"kind":"source_range_aggregate","proof_generated":false,"settlement_authority":false})
            );
        }
        Command::FoldRange {
            chain,
            prefix,
            range,
            out,
        } => {
            let programs = chain.programs()?;
            let prefix = ProofArtifact::read(&prefix)?;
            let range = ProofArtifact::read(&range)?;
            if prefix.statement().kind() != ProofKind::SourceTransition
                || range.statement().kind() != ProofKind::SourceRange
            {
                return Err("unexpected source prefix or range proof kind".into());
            }
            let witness = SourceWitness {
                policy: chain.policy(&programs),
                input: SourceInput::Fold {
                    previous: Box::new(SourceReceiptClaim::from_bytes(
                        prefix.statement().payload(),
                    )?),
                    range: Box::new(SourceRangeClaim::from_bytes(range.statement().payload())?),
                },
            };
            SourceJob::fold(&witness, &prefix, &range, &programs)?
                .export_public(&out, MAX_SOURCE_CYCLES)?;
            println!(
                "{}",
                serde_json::json!({"job":out,"kind":"source_prefix_fold","proof_generated":false,"settlement_authority":false})
            );
        }
        Command::Continue {
            chain,
            previous,
            prefix,
            block,
            out,
        } => {
            let programs = chain.programs()?;
            let previous = ProofArtifact::read(&previous)?;
            let claim = read_claim(&previous)?;
            let mut source = chain.source(&prefix, &programs)?;
            let blocks = block
                .iter()
                .map(|path| block_bytes(path))
                .collect::<Result<Vec<_>, _>>()?;
            let transition = source.record_transition(blocks)?;
            let witness = SourceWitness {
                policy: chain.policy(&programs),
                input: SourceInput::Advance {
                    previous: Box::new(claim),
                    transition: Box::new(transition),
                },
            };
            SourceJob::transition_with(&witness, Some(&previous), &programs)?
                .export_public(&out, 268_435_456)?;
            println!(
                "{}",
                serde_json::json!({"job":out,"height":source.height()?,"proof_generated":false})
            );
        }
        Command::Update {
            chain,
            block,
            proof,
            previous,
            previous_checkpoint,
            out,
        } => {
            let programs = chain.programs()?;
            let proof = ProofArtifact::read(&proof)?;
            let claim = read_claim(&proof)?;
            let source = chain.source(&block, &programs)?;
            if source.state_snapshot()?.commitment()? != claim.state {
                return Err("source prefix differs from its proof".into());
            }
            let checkpoint = source.checkpoint(chain.profile.confirmations)?;
            let previous = previous
                .as_ref()
                .map(|path| ProofArtifact::read(path))
                .transpose()?;
            let predecessor = match (&previous, previous_checkpoint) {
                (Some(artifact), Some(path)) => {
                    artifact.verify(artifact.statement())?;
                    if artifact.statement().kind() != ProofKind::Source {
                        return Err("expected a source registry update proof".into());
                    }
                    let old = SourceCheckpoint::from_bytes(&zutti_wallet::backup::read_private(
                        &path, 65_536,
                    )?)?;
                    Some(Box::new(PreviousUpdate {
                        claim: SourceUpdateClaim::from_bytes(artifact.statement().payload())?,
                        ancestry: source.prove_header(old.stable.height)?,
                        checkpoint: old,
                    }))
                }
                (None, None) => None,
                _ => return Err("previous proof and checkpoint must be supplied together".into()),
            };
            let witness = SourceUpdateWitness {
                policy: chain.policy(&programs),
                chain: claim,
                stable: source.prove_header(checkpoint.stable.height)?,
                previous: predecessor,
            };
            SourceJob::update_with(&witness, &proof, previous.as_ref(), &programs)?
                .export_public(&out, 268_435_456)?;
            zutti_wallet::backup::persist_new(
                &out.join("checkpoint.bin"),
                &checkpoint.to_bytes()?,
            )?;
            zutti_wallet::backup::persist_new(
                &out.join("state.bin"),
                &source.state_snapshot()?.to_bytes()?,
            )?;
            println!(
                "{}",
                serde_json::json!({"job":out,"checkpoint":checkpoint.id()?.to_string(),"stable_height":checkpoint.stable.height,"proof_generated":false})
            );
        }
        Command::InitStore { chain, database } => {
            let programs = chain.programs()?;
            let pipeline = SourcePipeline::create(
                &database,
                block_bytes(&chain.genesis)?,
                chain.policy(&programs),
                programs,
            )?;
            print_store(&pipeline)?;
        }
        Command::AdvanceStore {
            chain,
            database,
            block,
        } => {
            let programs = chain.programs()?;
            let mut pipeline = SourcePipeline::open(&database, chain.policy(&programs), programs)?;
            pipeline.advance(
                block
                    .iter()
                    .map(|path| block_bytes(path))
                    .collect::<Result<Vec<_>, _>>()?,
            )?;
            print_store(&pipeline)?;
        }
        Command::StoreStatus { chain, database } => {
            let programs = chain.programs()?;
            let pipeline = SourcePipeline::open(&database, chain.policy(&programs), programs)?;
            print_store(&pipeline)?;
        }
        Command::ExportStore {
            chain,
            database,
            out,
        } => {
            let programs = chain.programs()?;
            let pipeline = SourcePipeline::open(&database, chain.policy(&programs), programs)?;
            let job = pipeline.job()?;
            if out.exists() {
                let existing = SourceJob::read_public(&out)?;
                if !existing.matches(&job) {
                    return Err("existing job does not match pending source batch".into());
                }
            } else {
                job.export_public(&out, 268_435_456)?;
            }
            println!(
                "{}",
                serde_json::json!({"job": out, "proof_generated":false})
            );
        }
        Command::AcceptStore {
            chain,
            database,
            proof,
        } => {
            let programs = chain.programs()?;
            let mut pipeline = SourcePipeline::open(&database, chain.policy(&programs), programs)?;
            pipeline.accept(&ProofArtifact::read(&proof)?)?;
            print_store(&pipeline)?;
        }
        Command::ImportStore {
            chain,
            database,
            result,
            out,
        } => {
            let programs = chain.programs()?;
            let mut pipeline = SourcePipeline::open(&database, chain.policy(&programs), programs)?;
            let job = pipeline.job()?;
            let artifact = zutti_prover::remote::import_groth16(&result, job.statement())?;
            artifact.persist_new(&out)?;
            pipeline.accept(&artifact)?;
            print_store(&pipeline)?;
        }
        Command::Execute { job, prover } => println!(
            "{}",
            serde_json::to_string(
                &LocalProver::new(prover).execute_source(&SourceJob::read_public(&job)?)?
            )?
        ),
        Command::Prove { job, out, prover } => {
            let job = SourceJob::read_public(&job)?;
            let (statement, proof) = LocalProver::new(prover).prove_source(&job)?;
            ProofArtifact::new(statement, proof)?.persist_new(&out)?;
            println!(
                "{}",
                serde_json::json!({"proof":out,"proof_verified":true,"swap_executed":false})
            );
        }
        Command::Admit {
            profile,
            proof,
            state,
            checkpoint,
        } => {
            let profile =
                SourceAdmission::from_bytes(&zutti_wallet::backup::read_private(&profile, 4_096)?)?;
            let state =
                StateSnapshot::from_bytes(&zutti_wallet::backup::read_private(&state, 1_048_576)?)?;
            let checkpoint = SourceCheckpoint::from_bytes(&zutti_wallet::backup::read_private(
                &checkpoint,
                65_536,
            )?)?;
            let proof = ProofArtifact::read(&proof)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            let verified = profile.verify(&proof, &state, checkpoint, now)?;
            let checkpoint = verified.checkpoint(now)?;
            println!(
                "{}",
                serde_json::json!({"source_admitted":true,"height":verified.height(),"stable_height":checkpoint.stable.height,"timestamp":verified.timestamp(),"checkpoint":checkpoint.id()?.to_string(),"swap_executed":false})
            );
        }
        Command::Verify { proof } => {
            let artifact = ProofArtifact::read(&proof)?;
            if !matches!(
                artifact.statement().kind(),
                ProofKind::Source | ProofKind::SourceTransition | ProofKind::SourceRange
            ) {
                return Err("expected source evidence".into());
            }
            artifact.verify(artifact.statement())?;
            println!(
                "{}",
                serde_json::json!({"proof_verified":true,"range_only":artifact.statement().kind() == ProofKind::SourceRange,"program":artifact.statement().program().to_string(),"policy":artifact.statement().domain().to_string(),"current_chain_admitted":false,"swap_executed":false})
            );
        }
    }
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run(Args::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
