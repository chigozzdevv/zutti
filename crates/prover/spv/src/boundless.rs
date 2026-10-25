use crate::{Result, bounded, codec, ethereum, image, write_new};
use bincode::Options;
use clap::ValueEnum;
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver, Receipt};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::spv::{AdmissionWitness, MAX_WITNESS, Witness};
use zutti_wallet::{
    binding::{MAX_BINDING_WITNESS, PaymentBindingWitness},
    spv_pool::{MAX_SPV_POOL_WITNESS, SpvPoolWitness},
};

pub const MAX_CYCLES: u64 = 1 << 35;
const MAX_JOURNAL: usize = 65_664;
const MAX_STDIN: usize = 64 * 1024 * 1024 + 1044;
type Assumptions = Vec<(Digest, Vec<u8>)>;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Kind {
    Source,
    Admission,
    Binding,
    Pool,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Source => "spv_source",
            Self::Admission => "spv_admission",
            Self::Binding => "payment_binding",
            Self::Pool => "spv_pool",
        }
    }
    fn limit(self) -> usize {
        match self {
            Self::Source | Self::Admission => MAX_WITNESS,
            Self::Binding => MAX_BINDING_WITNESS,
            Self::Pool => MAX_SPV_POOL_WITNESS,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    version: u16,
    visibility: String,
    image_id: String,
    program: PathBuf,
    stdin: PathBuf,
    journal: PathBuf,
    cycles: u64,
    program_sha256: String,
    stdin_sha256: String,
    journal_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fulfillment {
    version: u16,
    chain_id: u64,
    request_id: String,
    image_id: String,
    journal: String,
    seal: String,
    input_visibility: String,
    elapsed_seconds: f64,
    request_to_receipt_seconds: u64,
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn regular(path: &Path, limit: usize) -> Result<Vec<u8>> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err("remote proving artifacts must be regular files".into());
    }
    bounded(path, limit)
}

fn expectation(kind: Kind, input: &[u8], program: Digest) -> Result<(Vec<u8>, Assumptions)> {
    Ok(match kind {
        Kind::Source => {
            let witness = Witness::from_bytes(input)?;
            if witness.policy().program() != program {
                return Err("source witness targets another program".into());
            }
            let required = witness
                .required_journal()?
                .into_iter()
                .map(|j| (program, j))
                .collect();
            (witness.evaluate()?.journal()?, required)
        }
        Kind::Admission => {
            let witness = AdmissionWitness::from_bytes(input)?;
            let previous = witness.predecessor();
            (
                witness.evaluate()?.to_bytes()?,
                vec![(previous.policy().program(), previous.to_bytes()?)],
            )
        }
        Kind::Binding => (
            PaymentBindingWitness::from_bytes(input)?
                .evaluate()?
                .to_bytes()?,
            vec![],
        ),
        Kind::Pool => {
            let witness = SpvPoolWitness::from_bytes(input)?;
            if witness.claim.pool_program != program {
                return Err("pool witness targets another program".into());
            }
            let result = witness.evaluate_with_assumptions()?;
            (result.claim.to_bytes()?, result.assumptions)
        }
    })
}

pub fn export(
    program_path: &Path,
    witness_path: &Path,
    receipts: &[PathBuf],
    kind: Kind,
    cycles: u64,
    out: &Path,
) -> Result<()> {
    if cycles == 0 || cycles > MAX_CYCLES || out.try_exists()? {
        return Err("invalid remote cycle limit or existing output".into());
    }
    let program = regular(program_path, 32 * 1024 * 1024)?;
    let image = image(&program)?;
    let input = regular(witness_path, kind.limit())?;
    let (journal, required) = expectation(kind, &input, image)?;
    if required.len() != receipts.len() {
        return Err("remote job requires the exact dependency receipts".into());
    }
    if !required.is_empty() {
        let manifest: serde_json::Value =
            serde_json::from_slice(&regular(&program_path.with_extension("json"), 4096)?)?;
        if manifest["composition"] != "inline-groth16"
            || manifest["kind"] != kind.name()
            || manifest["image_id"] != image.to_string()
            || manifest["program_sha256"] != sha(&program)
        {
            return Err(
                "recursive Boundless jobs require the matching inline-Groth16 guest build".into(),
            );
        }
    }
    let mut stdin = u32::try_from(input.len())?.to_le_bytes().to_vec();
    stdin.extend_from_slice(&input);
    for ((image, journal), path) in required.iter().zip(receipts) {
        let receipt: Receipt = codec().deserialize(&regular(path, crate::MAX_RECEIPT)?)?;
        stdin.extend_from_slice(&ethereum::seal(&receipt, *image, journal)?);
    }
    if journal.is_empty() || journal.len() > MAX_JOURNAL || stdin.len() > MAX_STDIN {
        return Err("remote proving artifacts exceed their bounds".into());
    }
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let target = parent.join(out.file_name().ok_or("invalid remote output path")?);
    let stage = tempfile::Builder::new()
        .prefix(".boundless-export-")
        .tempdir_in(&parent)?;
    let job = Job {
        version: 2,
        visibility: "public".into(),
        image_id: image.to_string(),
        program: target.join("program.elf"),
        stdin: target.join("stdin.bin"),
        journal: target.join("journal.bin"),
        cycles,
        program_sha256: sha(&program),
        stdin_sha256: sha(&stdin),
        journal_sha256: sha(&journal),
    };
    for (name, bytes) in [
        ("program.elf", program.as_slice()),
        ("stdin.bin", stdin.as_slice()),
        ("journal.bin", journal.as_slice()),
    ] {
        write_new(&stage.path().join(name), bytes)?;
    }
    write_new(
        &stage.path().join("job.json"),
        &serde_json::to_vec_pretty(&job)?,
    )?;
    fs::File::open(stage.path())?.sync_all()?;
    fs::rename(stage.path(), &target)?;
    fs::File::open(parent)?.sync_all()?;
    println!(
        "{}",
        serde_json::json!({"job":target.join("job.json"),"image_id":image.to_string(),"dependencies":required.len(),"cycles":cycles,"input_visibility":"public","paid_jobs":0})
    );
    Ok(())
}

struct Artifacts {
    job: Job,
    image: Digest,
    program: Vec<u8>,
    stdin: Vec<u8>,
    journal: Vec<u8>,
}
fn load(path: &Path) -> Result<Artifacts> {
    let job: Job = serde_json::from_slice(&regular(path, 16_384)?)?;
    if job.version != 2 || job.visibility != "public" || job.cycles == 0 || job.cycles > MAX_CYCLES
    {
        return Err("unsupported public remote job".into());
    }
    let program = regular(&job.program, 32 * 1024 * 1024)?;
    let stdin = regular(&job.stdin, MAX_STDIN)?;
    let journal = regular(&job.journal, MAX_JOURNAL)?;
    let image = image(&program)?;
    if image.to_string() != job.image_id
        || sha(&program) != job.program_sha256
        || sha(&stdin) != job.stdin_sha256
        || sha(&journal) != job.journal_sha256
    {
        return Err("remote proving artifacts changed".into());
    }
    Ok(Artifacts {
        job,
        image,
        program,
        stdin,
        journal,
    })
}

pub fn execute(path: &Path) -> Result<()> {
    let artifacts = load(path)?;
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&artifacts.stdin)
        .session_limit(Some(artifacts.job.cycles))
        .segment_limit_po2(20);
    let started = Instant::now();
    let session = ExternalProver::new("zutti-boundless-preflight", "r0vm")
        .execute(builder.build()?, &artifacts.program)?;
    if session.exit_code != risc0_zkvm::ExitCode::Halted(0)
        || session.journal.bytes != artifacts.journal
    {
        return Err("remote preflight journal or exit differs".into());
    }
    println!(
        "{}",
        serde_json::json!({"image_id":artifacts.image.to_string(),"cycles":session.cycles(),"seconds":started.elapsed().as_secs_f64(),"journal_matches":true,"host_assumptions":0,"proof_generated":false,"paid_jobs":0})
    );
    Ok(())
}

fn verified_fulfillment(proof: &Fulfillment, image: Digest, journal: &[u8]) -> Result<Receipt> {
    if proof.version != 1
        || proof.input_visibility != "public"
        || !matches!(proof.chain_id, 8453 | 84532 | 11155111)
        || proof.image_id != image.to_string()
        || hex::decode(&proof.journal)? != journal
        || !proof.elapsed_seconds.is_finite()
        || proof.elapsed_seconds < 0.0
    {
        return Err("Boundless fulfillment does not match the pinned public job".into());
    }
    ethereum::from_seal(image, journal, &hex::decode(&proof.seal)?)
}

pub fn import(job: &Path, proof: &Path, out: &Path) -> Result<()> {
    let artifacts = load(job)?;
    let proof: Fulfillment = serde_json::from_slice(&regular(proof, MAX_JOURNAL * 2 + 8192)?)?;
    let receipt = verified_fulfillment(&proof, artifacts.image, &artifacts.journal)?;
    let seal = ethereum::seal(&receipt, artifacts.image, &artifacts.journal)?;
    if out.try_exists()? {
        return Err("receipt output already exists".into());
    }
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".boundless-import-")
        .tempdir_in(parent)?;
    write_new(
        &stage.path().join("receipt.bin"),
        &codec().serialize(&receipt)?,
    )?;
    write_new(&stage.path().join("seal.bin"), &seal)?;
    let result = serde_json::json!({"image_id":artifacts.image.to_string(),"journal_sha256":sha(&artifacts.journal),"proof_verified":true,"chain_id":proof.chain_id,"request_id":proof.request_id,"collection_seconds":proof.elapsed_seconds,"request_to_receipt_seconds":proof.request_to_receipt_seconds,"canonicality_admitted":false,"swap_executed":false});
    write_new(
        &stage.path().join("result.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    fs::File::open(stage.path())?.sync_all()?;
    fs::rename(stage.path(), out)?;
    fs::File::open(parent)?.sync_all()?;
    println!("{result}");
    Ok(())
}

#[cfg(test)]
#[path = "../../methods/receipt.rs"]
mod dependency_verifier;

#[cfg(test)]
mod tests {
    use super::*;

    fn genuine() -> (Fulfillment, Digest, Vec<u8>) {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/pool-lifecycle.json"
        ))
        .unwrap();
        let operation = &fixture["operations"]["deposit"];
        let image = Digest(
            hex::decode(operation["image_id"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
        );
        let journal = hex::decode(operation["journal"].as_str().unwrap()).unwrap();
        let proof = serde_json::from_value(serde_json::json!({
            "version":1,"chain_id":11155111,"request_id":"0x01",
            "image_id":image.to_string(),"journal":hex::encode(&journal),
            "seal":operation["seal"],"input_visibility":"public",
            "elapsed_seconds":1.0,"request_to_receipt_seconds":1,
        }))
        .unwrap();
        (proof, image, journal)
    }

    #[test]
    fn genuine_groth16_fulfillment_and_guest_verifier_bind_the_exact_statement() {
        let (proof, image, journal) = genuine();
        let receipt = verified_fulfillment(&proof, image, &journal).unwrap();
        let seal = ethereum::seal(&receipt, image, &journal).unwrap();
        dependency_verifier::verify(image.0, &journal, &seal).unwrap();
        for field in 0..5 {
            let mut changed_image = image;
            let mut changed_journal = journal.clone();
            let mut changed_seal = seal.clone();
            match field {
                0 => changed_image.0[0] ^= 1,
                1 => changed_journal[0] ^= 1,
                2 => changed_seal[0] ^= 1,
                3 => changed_seal[100] ^= 1,
                _ => changed_seal.push(0),
            }
            assert!(ethereum::from_seal(changed_image, &changed_journal, &changed_seal).is_err());
            assert!(
                dependency_verifier::verify(changed_image.0, &changed_journal, &changed_seal)
                    .is_err()
            );
        }
    }

    #[test]
    fn fulfillment_cannot_change_visibility_network_or_job_statement() {
        for field in 0..6 {
            let (mut proof, image, journal) = genuine();
            match field {
                0 => proof.input_visibility = "private".into(),
                1 => proof.chain_id = 1,
                2 => proof.image_id = "00".repeat(32),
                3 => proof.journal.push_str("00"),
                4 => proof.version = 2,
                _ => proof.elapsed_seconds = -1.0,
            }
            assert!(verified_fulfillment(&proof, image, &journal).is_err());
        }
    }

    #[test]
    fn large_public_jobs_keep_explicit_cycle_bounds() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/genesis-coverage");
        let directory = tempfile::tempdir().unwrap();
        let out = directory.path().join("job");
        export(
            &root.join("program.elf"),
            &root.join("witness.bin"),
            &[],
            Kind::Source,
            32_000_000_000,
            &out,
        )
        .unwrap();
        assert_eq!(
            load(&out.join("job.json")).unwrap().job.cycles,
            32_000_000_000
        );
        let rejected = directory.path().join("rejected");
        assert!(
            export(
                &root.join("program.elf"),
                &root.join("witness.bin"),
                &[],
                Kind::Source,
                MAX_CYCLES + 1,
                &rejected,
            )
            .is_err()
        );
        assert!(!rejected.exists());
        let path = out.join("job.json");
        let mut job: Job = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        job.cycles = MAX_CYCLES + 1;
        fs::write(&path, serde_json::to_vec(&job).unwrap()).unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn public_export_frames_the_canonical_witness_and_rejects_artifact_changes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/genesis-coverage");
        let directory = tempfile::tempdir().unwrap();
        let out = directory.path().join("job");
        export(
            &root.join("program.elf"),
            &root.join("witness.bin"),
            &[],
            Kind::Source,
            1_000_000,
            &out,
        )
        .unwrap();
        let artifacts = load(&out.join("job.json")).unwrap();
        let witness = fs::read(root.join("witness.bin")).unwrap();
        assert_eq!(&artifacts.stdin[..4], &(witness.len() as u32).to_le_bytes());
        assert_eq!(&artifacts.stdin[4..], witness);
        assert_eq!(
            artifacts.journal,
            fs::read(root.join("journal.bin")).unwrap()
        );
        assert!(
            export(
                &root.join("program.elf"),
                &root.join("witness.bin"),
                &[],
                Kind::Source,
                1,
                &out
            )
            .is_err()
        );
        fs::write(out.join("stdin.bin"), b"changed").unwrap();
        assert!(load(&out.join("job.json")).is_err());
        let proof = directory.path().join("proof.json");
        fs::write(&proof, b"{}").unwrap();
        assert!(
            import(
                &out.join("job.json"),
                &proof,
                &directory.path().join("receipt")
            )
            .is_err()
        );
        assert!(!directory.path().join("receipt").exists());
    }

    #[test]
    fn recursive_export_rejects_native_assumptions_and_non_groth16_receipts() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/genesis-coverage");
        let directory = tempfile::tempdir().unwrap();
        let program = fs::read(root.join("program.elf")).unwrap();
        write_new(&directory.path().join("program.elf"), &program).unwrap();
        let previous = zutti_source::spv::HeaderClaim::from_bytes(
            &fs::read(root.join("journal.bin")).unwrap(),
        )
        .unwrap();
        let header =
            hex::decode(include_str!("../../../../tests/fixtures/spv/testnet-1.hex").trim())
                .unwrap();
        let witness = Witness::Headers {
            previous,
            headers: vec![header[..zutti_source::spv::HEADER_BYTES].to_vec()],
        };
        let input = directory.path().join("witness.bin");
        write_new(&input, &witness.to_bytes().unwrap()).unwrap();
        let manifest = directory.path().join("program.json");
        for composition in ["native", "inline-groth16"] {
            fs::write(&manifest, serde_json::to_vec(&serde_json::json!({"kind":"spv_source","composition":composition,"image_id":image(&program).unwrap().to_string(),"program_sha256":sha(&program)})).unwrap()).unwrap();
            let out = directory.path().join("job");
            let error = export(
                &directory.path().join("program.elf"),
                &input,
                &[root.join("receipt.bin")],
                Kind::Source,
                1_000_000,
                &out,
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains(if composition == "native" {
                    "matching inline-Groth16"
                } else {
                    "genuine Groth16"
                }),
                "{error}"
            );
            assert!(!out.exists());
        }
    }
}
