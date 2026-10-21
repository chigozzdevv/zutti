use crate::{
    Result, archive,
    args::{ExportArgs, RecordArgs, RestoreArgs},
};
use zutti_protocol::{Digest, Secret32, encoding::Canonical};
use zutti_wallet::{pool::PoolWitness, swap::SwapRecord};

pub fn export(args: ExportArgs) -> Result<()> {
    let (domain, key, plaintext) =
        archive::open(&args.archive, zutti_wallet::pool::MAX_POOL_WITNESS)?;
    let witness = PoolWitness::from_bytes(&plaintext)?;
    let record = SwapRecord::from_reservation(&witness)?;
    if record.domain() != domain {
        return Err("reservation domain mismatch".into());
    }
    record.save_new(&args.out, &key)?;
    println!(
        "swap_record_saved=true swap_id={} settlement_authority=false",
        record.id()?
    );
    Ok(())
}

pub fn load(args: &RecordArgs) -> Result<(SwapRecord, Secret32)> {
    let domain = Digest::from_hex(&args.archive.domain)?;
    let key = archive::key(&args.archive.key_file)?;
    let record = SwapRecord::read(
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
