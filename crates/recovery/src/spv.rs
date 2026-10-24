use crate::{
    Result, archive,
    args::{ExportArgs, PrepareArgs, RecordArgs, RestoreArgs},
};
use zeroize::Zeroizing;
use zutti_protocol::{Digest, Secret32, encoding::Canonical};
use zutti_wallet::{
    backup,
    spv_pool::{
        MAX_SPV_POOL_WITNESS, SpvPoolWitness,
        recovery::{SpvSettlementEvidence, SpvSwapRecord},
    },
};

pub fn export(args: ExportArgs) -> Result<()> {
    let (domain, key, plaintext) = archive::open(&args.archive, MAX_SPV_POOL_WITNESS)?;
    let witness = SpvPoolWitness::from_bytes(&plaintext)?;
    let record = SpvSwapRecord::from_reservation(&witness)?;
    if record.domain() != domain {
        return Err("SPV reservation domain mismatch".into());
    }
    record.save_new(&args.out, &key)?;
    println!(
        "swap_record_saved=true swap_id={} settlement_authority=false",
        record.id()?
    );
    Ok(())
}
fn load(args: &RecordArgs) -> Result<(SpvSwapRecord, Secret32)> {
    let domain = Digest::from_hex(&args.archive.domain)?;
    let key = archive::key(&args.archive.key_file)?;
    let record = SpvSwapRecord::read(
        &args.archive.backup,
        &key,
        domain,
        Digest::from_hex(&args.swap)?,
    )?;
    Ok((record, key))
}
pub fn restore(args: RestoreArgs) -> Result<()> {
    let (record, key) = load(&args.record)?;
    record.save_new(&args.out, &key)?;
    println!("swap_record_restored=true settlement_authority=false");
    Ok(())
}
pub fn prepare(args: PrepareArgs) -> Result<()> {
    let (record, key) = load(&args.record)?;
    let archive = backup::read_private(&args.evidence, 4224)?;
    let plaintext = backup::open(record.domain(), &archive, &key)?;
    let evidence = SpvSettlementEvidence::from_bytes(&plaintext)?;
    let witness = record.settlement(evidence.root, evidence.path, evidence.window)?;
    let bytes = Zeroizing::new(witness.to_bytes()?);
    backup::persist_new(&args.out, &backup::seal(record.domain(), &bytes, &key)?)?;
    println!(
        "witness_prepared=true proof_generated=false transaction_submitted=false settlement_authority=false"
    );
    Ok(())
}
