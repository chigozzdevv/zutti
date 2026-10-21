use crate::{Result, args::PrepareArgs, restore};
use zeroize::Zeroizing;
use zutti_protocol::encoding::Canonical;
use zutti_wallet::{
    backup,
    swap::{MAX_SETTLEMENT_EVIDENCE, SettlementAction, SettlementEvidence},
};

pub fn prepare(args: PrepareArgs, action: SettlementAction) -> Result<()> {
    let (record, key) = restore::load(&args.record)?;
    let archive = backup::read_private(&args.evidence, MAX_SETTLEMENT_EVIDENCE + 128)?;
    let plaintext = backup::open(record.domain(), &archive, &key)?;
    let evidence = SettlementEvidence::from_bytes(&plaintext)?;
    let witness = record.settlement(action, &evidence)?;
    let bytes = Zeroizing::new(witness.to_bytes()?);
    let archive = backup::seal(record.domain(), &bytes, &key)?;
    backup::persist_new(&args.out, &archive)?;
    println!(
        "witness_prepared=true proof_generated=false transaction_submitted=false settlement_authority=false"
    );
    Ok(())
}
