use crate::{
    Error, Result,
    binding::PaymentBindingWitness,
    pool::{NoteOutput, escrow::EscrowNote},
    preparation::PreparationWitness,
    private_note::NoteEncryption,
    source_notes::SourceScan,
    spv_pool::SpvReservation,
};
use orchard::keys::FullViewingKey;
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest, Secret32,
    amount::{Amount, FeeBps},
    domain::Domain,
    encoding::{Canonical, Reader, Writer},
    order::{Direction, OrderInput, OrderTerms},
    policy::PaymentWindow,
    private_note::{NoteRecipient, PrivateNote},
};
use zutti_source::spv::{HeaderClaim, Policy};

#[derive(Debug)]
pub struct QuoteTerms {
    pub offer: Digest,
    pub zec: Amount,
    pub gross_usdc: Amount,
    pub fee_bps: FeeBps,
    pub fee_caps: [Amount; 3],
    pub nonce: Digest,
}

#[derive(Debug)]
pub struct MakerRecipients {
    pub zcash: [u8; 43],
    pub refund: NoteRecipient,
}
impl MakerRecipients {
    pub fn from_address(address: &str, refund: NoteRecipient) -> Result<Self> {
        use zcash_address::unified::{Address, Container, Encoding, Receiver};
        let (network, revision, address) = Address::decode(address).map_err(|_| Error::Payment)?;
        if network != zcash_protocol::consensus::NetworkType::Test
            || revision != zcash_protocol::address::Revision::R0
        {
            return Err(Error::Payment);
        }
        let zcash = address
            .items()
            .into_iter()
            .find_map(|receiver| match receiver {
                Receiver::Orchard(value) => Some(value),
                _ => None,
            })
            .ok_or(Error::Payment)?;
        Ok(Self { zcash, refund })
    }
}

pub struct OrderContext<'a> {
    pub domain: Domain,
    pub policy: Policy,
    pub anchor: HeaderClaim,
    pub source: &'a SourceScan,
    pub viewing: &'a FullViewingKey,
    pub payout: NoteRecipient,
}
impl core::fmt::Debug for OrderContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OrderContext([REDACTED])")
    }
}

pub struct PreparedOrder {
    reservation: SpvReservation,
    pczt: Zeroizing<Vec<u8>>,
}

impl core::fmt::Debug for PreparedOrder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PreparedOrder([REDACTED])")
    }
}

impl PreparedOrder {
    pub fn prepare(
        context: OrderContext<'_>,
        quote: QuoteTerms,
        maker: MakerRecipients,
    ) -> Result<Self> {
        let OrderContext {
            domain,
            policy,
            anchor,
            source,
            viewing,
            payout,
        } = context;
        if !policy.paid_only()
            || anchor.policy() != policy
            || domain.source_policy() != policy.id()?
            || anchor.height().map_err(|_| Error::Payment)? != source.height
            || anchor.tip().map_err(|_| Error::Payment)? != source.block
        {
            return Err(Error::Payment);
        }
        let amount = quote.gross_usdc.net_after_fee(quote.fee_bps);
        let id = domain.id()?;
        let output = |recipient| -> Result<NoteOutput> {
            Ok(NoteOutput {
                note: PrivateNote::new(
                    id,
                    recipient,
                    amount,
                    Secret32::random()?,
                    Secret32::random()?,
                )?,
                encryption: NoteEncryption::random()?,
            })
        };
        let payout = output(payout)?;
        let refund = output(maker.refund)?;
        let expiry = source
            .height
            .checked_add(policy.window_blocks())
            .ok_or(Error::Payment)?;
        let terms = OrderTerms::new(OrderInput {
            domain,
            direction: Direction::ZecToUsdc,
            offer: quote.offer,
            zec: quote.zec,
            usdc: amount,
            receiver: maker.zcash,
            payout: payout.fixed()?,
            refund: refund.fixed()?,
            gross_output: quote.gross_usdc,
            fee_bps: quote.fee_bps,
            fee_caps: quote.fee_caps,
            window: PaymentWindow::new(0, source.height, source.block, expiry)?,
            nonce: quote.nonce,
        })?;
        let order_randomness = Secret32::random()?;
        let reservation_randomness = Secret32::random()?;
        let pczt = Zeroizing::new(source.prepare_payment(viewing, &terms, &order_randomness)?);
        let inspected = crate::prepare::inspect_ironwood(&pczt, &terms, &order_randomness)?;
        let binding = PaymentBindingWitness::new(
            policy,
            PreparationWitness::new(
                terms.clone(),
                Secret32::from_bytes(*order_randomness.expose())?,
                Secret32::from_bytes(*reservation_randomness.expose())?,
                pczt.to_vec(),
            )?,
        )?
        .evaluate()?;
        let escrow = EscrowNote::new(
            id,
            binding.order(),
            binding.reservation(),
            amount.units(),
            Secret32::random()?,
            Secret32::random()?,
        )?;
        let mut nonce = [0; 24];
        getrandom::fill(&mut nonce).map_err(|_| zutti_protocol::Error::Entropy)?;
        let reservation = SpvReservation {
            terms,
            order_randomness,
            reservation_randomness,
            txid: inspected.txid(),
            escrow,
            nonce,
            anchor,
            binding,
            payout,
            refund,
        };
        reservation.validate(policy, id)?;
        Ok(Self { reservation, pczt })
    }

    pub fn binding_witness(&self) -> Result<PaymentBindingWitness> {
        let res = &self.reservation;
        PaymentBindingWitness::new(
            res.anchor.policy(),
            PreparationWitness::new(
                res.terms.clone(),
                Secret32::from_bytes(*res.order_randomness.expose())?,
                Secret32::from_bytes(*res.reservation_randomness.expose())?,
                self.pczt.to_vec(),
            )?,
        )
    }

    pub fn reservation(&self) -> &SpvReservation {
        &self.reservation
    }
    pub fn payment(&self) -> &[u8] {
        &self.pczt
    }
}

impl Canonical for PreparedOrder {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIORD");
        writer.u16(1);
        self.reservation.encode(writer)?;
        writer.bytes(&self.pczt)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIORD" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let reservation = SpvReservation::decode(reader)?;
        reservation.validate(
            reservation.anchor.policy(),
            reservation.terms.domain().id()?,
        )?;
        let pczt = Zeroizing::new(reader.bytes(crate::prepare::MAX_PCZT)?.to_vec());
        let payment = crate::prepare::inspect_ironwood(
            &pczt,
            &reservation.terms,
            &reservation.order_randomness,
        )
        .map_err(|_| zutti_protocol::Error::Invalid("prepared order payment"))?;
        if payment.txid() != reservation.txid {
            return Err(zutti_protocol::Error::Invalid(
                "prepared order transaction identity",
            ));
        }
        Ok(Self { reservation, pczt })
    }
}
