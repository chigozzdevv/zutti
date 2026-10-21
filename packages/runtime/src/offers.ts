import { ZuttiError, wire_offer } from "@zutti/schema";
import type { IntentInput, Offer, RfqDomain, WireOffer } from "@zutti/schema";
import type { ProtocolCore } from "@zutti/bindings";
import { canonical, digest, immutable } from "./encoding.js";

export async function verify_offer(
  value: unknown,
  request: IntentInput,
  intent_id: string,
  request_id: string,
  maker_id: string,
  domain: RfqDomain,
  core: ProtocolCore,
  now: number,
  request_deadline: number,
): Promise<WireOffer> {
  const offer = wire_offer.parse(value);
  const { id, ...body } = offer;
  if (
    id !== (await digest("offer", body)) ||
    offer.intent_id !== intent_id ||
    offer.request_id !== request_id ||
    offer.maker_id !== maker_id ||
    offer.deadline <= now ||
    offer.deadline > request_deadline ||
    offer.input.asset !== request.from ||
    offer.output.asset !== request.to ||
    offer.gross_output.asset !== request.to ||
    offer.maker_fee.amount.asset !== request.to ||
    canonical(offer.policy) !== canonical(domain.policy) ||
    core.compare(request.from, offer.input.amount, request.amount) !== 0
  )
    throw new ZuttiError("invalid_message", "Offer does not match the retained request");
  core.amount(offer.input.asset, offer.input.amount);
  const amounts = core.net(request.to, offer.gross_output.amount, offer.maker_fee.bps);
  if (
    core.compare(request.to, amounts.net, offer.output.amount) !== 0 ||
    core.compare(request.to, amounts.fee, offer.maker_fee.amount.amount) !== 0 ||
    (request.min_output !== undefined &&
      core.compare(request.to, offer.output.amount, request.min_output) < 0)
  )
    throw new ZuttiError("invalid_message", "Offer amount or fee arithmetic does not match");
  const fees = new Set<string>();
  for (const fee of offer.fees) {
    const key = `${fee.kind}/${fee.asset}`;
    core.amount(fee.asset, fee.max_amount, false);
    if (
      fees.has(key) ||
      (fee.kind === "source_network" && fee.asset !== "ZEC") ||
      (fee.kind === "ethereum_execution" && fee.asset !== "ETH") ||
      (fee.paid_by === "user" && fee.pay_from === "maker_inventory") ||
      (fee.paid_by === "maker" && fee.pay_from !== "maker_inventory")
    )
      throw new ZuttiError("invalid_message", "Offer fee terms are invalid");
    fees.add(key);
    if (
      fee.estimated_amount !== null &&
      core.compare(
        fee.asset,
        core.amount(fee.asset, fee.estimated_amount, false).amount,
        fee.max_amount,
      ) > 0
    )
      throw new ZuttiError("invalid_message", "Fee estimate exceeds its signed cap");
  }
  if (
    !offer.fees.some((fee) => fee.kind === "source_network") ||
    !offer.fees.some((fee) => fee.kind === "ethereum_execution")
  )
    throw new ZuttiError("invalid_message", "Offer omits network fee terms");
  return offer;
}
export function public_offer(value: WireOffer): Offer {
  const { nonce: _nonce, request_id: _request, ...offer } = value;
  return immutable({ ...structuredClone(offer), authentication: "verified" as const });
}
