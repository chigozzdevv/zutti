use clap::Parser;
use halo2_proofs::poly::commitment::Params;
use orchard::circuit::{OrchardCircuitVersion, VerifyingKey};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::Instant};
use zutti_validation_runner::Result;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    out: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    fs::create_dir(&args.out)?;
    let started = Instant::now();
    let params = Params::<pasta_curves::vesta::Affine>::new(11);
    let params_seconds = started.elapsed().as_secs_f64();
    let mut bytes = Vec::new();
    params.write(&mut bytes)?;
    fs::write(args.out.join("vesta-k11.params"), &bytes)?;
    let mut read = bytes.as_slice();
    let decoded = Params::<pasta_curves::vesta::Affine>::read(&mut read)?;
    let mut roundtrip = Vec::new();
    decoded.write(&mut roundtrip)?;
    if !read.is_empty() || bytes != roundtrip {
        return Err("public parameter serialization mismatch".into());
    }
    let mut cases = Vec::new();
    for version in [
        OrchardCircuitVersion::InsecurePreNu6_2,
        OrchardCircuitVersion::FixedPostNu6_2,
        OrchardCircuitVersion::PostNu6_3,
    ] {
        let started = Instant::now();
        let key = VerifyingKey::build(version);
        if key.circuit_version() != version {
            return Err("verification key circuit mismatch".into());
        }
        std::hint::black_box(key);
        let entry = serde_json::json!({"circuit_version":format!("{version:?}"),
            "native_key_build_seconds":started.elapsed().as_secs_f64()});
        println!("{entry}");
        cases.push(entry);
    }
    let report = serde_json::json!({"schema":1,"native_only":true,"vm_cycles_measured":false,
        "k":11,"parameter_bytes":bytes.len(),"parameter_sha256":hex::encode(Sha256::digest(&bytes)),
        "native_parameter_build_seconds":params_seconds,"parameter_roundtrip_matches":true,
        "cases":cases,"proof_generated":false,"paid_jobs":0});
    fs::write(
        args.out.join("timings.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
