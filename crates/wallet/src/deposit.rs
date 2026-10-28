use crate::{
    pool::NoteOutput,
    private_note::NoteEncryption,
    spv_pool::{SpvAction, SpvPoolClaim, SpvPoolWitness},
};
use zutti_protocol::{
    Digest, Error, Result, Secret32,
    amount::{Amount, Asset},
    domain::Domain,
    network::ZCASH_TESTNET_GENESIS,
    pool::{OperationKind, PoolClaim, nullifier_digest, output_digest},
    private_note::{NoteRecipient, PrivateNote},
};
use zutti_source::spv::Policy;

#[cfg(feature = "native")]
use crate::private_note::NoteKeys;

#[derive(Debug)]
pub struct DepositContext {
    domain: Domain,
    policy: Policy,
    binding_program: Digest,
    pool_program: Digest,
    owner: [u8; 20],
}

impl DepositContext {
    pub fn new(
        domain: Domain,
        policy: Policy,
        binding_program: Digest,
        pool_program: Digest,
        owner: [u8; 20],
    ) -> Result<Self> {
        if domain.source_genesis() != ZCASH_TESTNET_GENESIS
            || domain.destination_chain() != 11155111
            || domain.source_policy() != policy.id().map_err(|_| Error::Invalid("source policy"))?
            || domain.payment_profile() != crate::binding::payment_profile()
            || owner == [0; 20]
        {
            return Err(Error::Invalid("Testnet deposit context"));
        }
        Digest::nonzero(binding_program.0)?;
        Digest::nonzero(pool_program.0)?;
        Ok(Self {
            domain,
            policy,
            binding_program,
            pool_program,
            owner,
        })
    }

    #[cfg(feature = "native")]
    pub fn prepare(&self, keys: &NoteKeys, amount: Amount) -> Result<SpvPoolWitness> {
        self.prepare_for(
            keys.recipient()?,
            amount,
            Secret32::random()?,
            Secret32::random()?,
            NoteEncryption::random()?,
        )
    }

    pub fn prepare_for(
        &self,
        recipient: NoteRecipient,
        amount: Amount,
        rho: Secret32,
        blinding: Secret32,
        encryption: NoteEncryption,
    ) -> Result<SpvPoolWitness> {
        if amount.asset() != Asset::Usdc {
            return Err(Error::Asset);
        }
        let value = u64::try_from(amount.positive()?.units()).map_err(|_| Error::Overflow)?;
        let domain = self.domain.id()?;
        let output = NoteOutput {
            note: PrivateNote::new(domain, recipient, amount, rho, blinding)?,
            encryption,
        };
        let witness = SpvPoolWitness {
            policy: self.policy,
            claim: SpvPoolClaim {
                policy: self.domain.source_policy(),
                binding_program: self.binding_program,
                pool_program: self.pool_program,
                expiry: 0,
                pool: PoolClaim {
                    kind: OperationKind::Deposit,
                    domain,
                    root: Digest::default(),
                    checkpoint: Digest::default(),
                    order_tag: Digest::default(),
                    order: Digest::default(),
                    reservation: Digest::default(),
                    value,
                    owner: self.owner,
                    nullifiers: nullifier_digest(&[])?,
                    outputs: output_digest(&[output.binding()?])?,
                },
            },
            inputs: vec![],
            outputs: vec![output],
            action: SpvAction::Deposit,
        };
        witness.evaluate()?;
        Ok(witness)
    }
}
