import { z } from "zod";
import {
  MAX_MAKERS,
  ZuttiError,
  decline,
  digest as id_schema,
  envelope,
  intent_input,
  maker_announcement,
  request_body,
  require_pair,
  rfq_domain,
} from "@zutti/schema";
import type {
  IntentInput,
  MakerAnnouncement,
  OfferCollection,
  RfqDomain,
  WireOffer,
} from "@zutti/schema";
import type { ProtocolCore } from "@zutti/bindings";
import { canonical, immutable, random_id } from "./encoding.js";
import { MessagingIdentity } from "./identity.js";
import {
  authorize_poll,
  open_message,
  seal_message,
  verify_announcement,
  verify_envelope,
} from "./messages.js";
import { public_offer, verify_offer } from "./offers.js";
import type { PrivateStore } from "./store.js";
import type { RelayTransport } from "./transport.js";

const plan_schema = z.strictObject({
  version: z.literal(1),
  intent_id: id_schema,
  request: intent_input,
  identity: z.strictObject({
    signing: z.record(z.string(), z.unknown()),
    encryption: z.record(z.string(), z.unknown()),
  }),
  selected: z
    .array(z.strictObject({ maker: maker_announcement, message: envelope }))
    .max(MAX_MAKERS),
});
type Plan = z.infer<typeof plan_schema>;
export interface ClientRuntimeConfig {
  domain: RfqDomain;
  core: ProtocolCore;
  store: PrivateStore;
  relay: RelayTransport;
  maker_ids: readonly string[];
  collection_ms?: number;
}
export class RfqClient {
  readonly domain: Readonly<RfqDomain>;
  private readonly flights = new Map<string, Promise<OfferCollection>>();
  private readonly config: ClientRuntimeConfig;
  constructor(config: ClientRuntimeConfig) {
    if (
      !Array.isArray(config.maker_ids) ||
      config.maker_ids.length === 0 ||
      config.maker_ids.length > 64
    )
      throw new ZuttiError(
        "wallet_unavailable",
        "Owner runtime must select the permitted maker identities",
      );
    for (const id of config.maker_ids) id_schema.parse(id);
    const budget = config.collection_ms ?? 5_000;
    if (!Number.isInteger(budget) || budget < 1 || budget > 15_000)
      throw new ZuttiError("invalid_input", "Invalid collection budget");
    this.config = { ...config, maker_ids: Object.freeze([...config.maker_ids]) };
    this.domain = immutable(rfq_domain.parse(config.domain));
  }
  normalize(input: IntentInput): Readonly<IntentInput> {
    const request = intent_input.parse(input);
    require_pair(this.domain.policy, request.from, request.to);
    const result = {
      ...request,
      amount: this.config.core.amount(request.from, request.amount).amount,
    };
    if (result.min_output !== undefined)
      result.min_output = this.config.core.amount(result.to, result.min_output).amount;
    return immutable(result);
  }
  async offers(
    intent_id: string,
    input: IntentInput,
    signal?: AbortSignal,
  ): Promise<OfferCollection> {
    id_schema.parse(intent_id);
    if (signal?.aborted) throw new ZuttiError("aborted", "RFQ collection was interrupted");
    if (this.flights.has(intent_id))
      throw new ZuttiError("service_unavailable", "This intent already has an active collection");
    const flight = this.collect(intent_id, this.normalize(input), signal);
    this.flights.set(intent_id, flight);
    try {
      return await flight;
    } finally {
      this.flights.delete(intent_id);
    }
  }
  private async plan(intent_id: string, request: IntentInput, signal?: AbortSignal): Promise<Plan> {
    const key = `intent-${intent_id}`;
    const saved = await this.config.store.read(key);
    if (saved !== undefined) return this.validate_plan(saved, intent_id, request);
    const makers: MakerAnnouncement[] = [];
    for (const value of await this.config.relay.makers(signal)) {
      try {
        const maker = await verify_announcement(value, this.domain, Date.now());
        if (
          maker.pairs.includes(`${request.from}/${request.to}` as "ZEC/USDC" | "USDC/ZEC") &&
          this.config.maker_ids.includes(maker.maker_id) &&
          !makers.some((item) => item.maker_id === maker.maker_id)
        )
          makers.push(maker);
      } catch {
        continue;
      }
    }
    makers.sort((left, right) => left.maker_id.localeCompare(right.maker_id));
    const identity = await MessagingIdentity.create();
    const selected: Plan["selected"] = [];
    for (const maker of makers.slice(0, MAX_MAKERS)) {
      const request_id = random_id();
      const deadline = Date.now() + 15_000;
      const body = request_body.parse({
        version: 1,
        intent_id,
        request_id,
        request,
        reply_key: identity.encryption_key,
        policy: this.domain.policy,
        deadline,
      });
      const message = await seal_message(
        identity,
        {
          network: this.domain.network,
          deployment_id: this.domain.deployment_id,
          type: "request",
          request_id,
          recipient_id: maker.maker_id,
          recipient_key: maker.encryption_key,
          deadline,
        },
        body,
      );
      selected.push({ maker, message });
    }
    const plan: Plan = {
      version: 1,
      intent_id,
      request,
      identity: identity.export() as Plan["identity"],
      selected,
    };
    await this.config.store.create(key, plan);
    return this.validate_plan(await this.config.store.read(key), intent_id, request);
  }
  private validate_plan(value: unknown, id: string, request: IntentInput): Plan {
    const plan = plan_schema.parse(value);
    if (
      plan.intent_id !== id ||
      canonical(plan.request) !== canonical(request) ||
      plan.selected.some((value) => !this.config.maker_ids.includes(value.maker.maker_id))
    )
      throw new ZuttiError("persistence_failure", "Saved intent does not match its request");
    return plan;
  }
  private async response(
    plan: Plan,
    value: unknown,
    identity: MessagingIdentity,
    now: number,
  ): Promise<{ maker: string; offer?: WireOffer }> {
    const message = await verify_envelope(value, this.domain, now);
    const selected = plan.selected.find(
      (item) =>
        item.maker.maker_id === message.sender_id && item.message.request_id === message.request_id,
    );
    if (
      !selected ||
      message.sender_key !== selected.maker.signing_key ||
      message.deadline > selected.message.deadline
    )
      throw new ZuttiError("invalid_message", "Unexpected RFQ response");
    const payload = await open_message(identity, message, this.domain, now);
    if (message.type === "decline") {
      const declined = decline.parse(payload);
      if (declined.request_id !== message.request_id)
        throw new ZuttiError("invalid_message", "Decline does not match its request");
      return { maker: selected.maker.maker_id };
    }
    if (message.type !== "offer")
      throw new ZuttiError("invalid_message", "Unexpected response type");
    const offer = await verify_offer(
      payload,
      plan.request,
      plan.intent_id,
      message.request_id,
      selected.maker.maker_id,
      this.domain,
      this.config.core,
      now,
      selected.message.deadline,
    );
    if (offer.deadline !== message.deadline)
      throw new ZuttiError("invalid_message", "Offer expiry does not match its envelope");
    return { maker: selected.maker.maker_id, offer };
  }
  private async collect(
    intent_id: string,
    request: IntentInput,
    signal?: AbortSignal,
  ): Promise<OfferCollection> {
    const plan = await this.plan(intent_id, request, signal);
    const identity = await MessagingIdentity.restore(plan.identity);
    const responses = new Map<string, WireOffer | undefined>();
    const retained = async (maker: string): Promise<void> => {
      const message = await this.config.store.read(`response-${intent_id}-${maker}`);
      if (message === undefined) return;
      try {
        const result = await this.response(plan, message, identity, Date.now());
        responses.set(result.maker, result.offer);
      } catch {
        return;
      }
    };
    for (const item of plan.selected) {
      await retained(item.maker.maker_id);
      if (!responses.has(item.maker.maker_id) && item.message.deadline > Date.now()) {
        try {
          await this.config.relay.send(item.message, signal);
        } catch (error) {
          if (signal?.aborted) throw error;
        }
      }
    }
    const budget = this.config.collection_ms ?? 5_000;
    if (!Number.isInteger(budget) || budget < 1 || budget > 15_000)
      throw new ZuttiError("invalid_input", "Invalid collection budget");
    const until = performance.now() + budget;
    while (responses.size < plan.selected.length && performance.now() < until) {
      if (signal?.aborted) throw new ZuttiError("aborted", "RFQ collection was interrupted");
      try {
        for (const value of await this.config.relay.poll(
          await authorize_poll(identity, this.domain, Date.now()),
          signal,
        )) {
          try {
            const result = await this.response(plan, value, identity, Date.now());
            const key = `response-${intent_id}-${result.maker}`;
            await this.config.store.create(key, value);
            await retained(result.maker);
          } catch (error) {
            if (error instanceof ZuttiError && error.code === "persistence_failure") throw error;
          }
        }
      } catch (error) {
        if (
          signal?.aborted ||
          (error instanceof ZuttiError && error.code === "persistence_failure")
        )
          throw error;
      }
      if (responses.size < plan.selected.length)
        await new Promise((resolve) => setTimeout(resolve, 100));
    }
    const now = Date.now();
    const items = [...responses.values()]
      .filter((value): value is WireOffer => value !== undefined && value.deadline > now)
      .map(public_offer);
    return immutable({
      intent_id,
      items,
      requested_makers: plan.selected.map((value) => value.maker.maker_id),
      responded_makers: [...responses.keys()],
      collection: responses.size === plan.selected.length ? "complete" : "timed_out",
      collected_at: now,
    });
  }
  async select(intent_id: string, input: IntentInput, offer_id: string): Promise<WireOffer> {
    id_schema.parse(offer_id);
    const plan = this.validate_plan(
      await this.config.store.read(`intent-${intent_id}`),
      intent_id,
      this.normalize(input),
    );
    const identity = await MessagingIdentity.restore(plan.identity);
    for (const item of plan.selected) {
      const saved = await this.config.store.read(`response-${intent_id}-${item.maker.maker_id}`);
      if (saved === undefined) continue;
      try {
        const result = await this.response(plan, saved, identity, Date.now());
        if (result.offer?.id === offer_id) return result.offer;
      } catch {
        continue;
      }
    }
    throw new ZuttiError(
      "offer_unavailable",
      "Offer is missing, expired or belongs to another intent",
    );
  }
}
