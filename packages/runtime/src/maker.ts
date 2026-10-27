import { z } from "zod";
import {
  ZuttiError,
  envelope,
  maker_start_options,
  request_body,
  rfq_domain,
  require_pair,
} from "@zutti/schema";
import type {
  FeeQuote,
  IntentInput,
  MakerAnnouncement,
  MakerStartOptions,
  OfferBody,
  RfqDomain,
  ReceivingDescriptor,
} from "@zutti/schema";
import type { ProtocolCore } from "@zutti/bindings";
import { canonical, digest, random_id } from "./encoding.js";
import { verify_offer } from "./offers.js";
import { MessagingIdentity } from "./identity.js";
import {
  announce,
  authorize_poll,
  open_message,
  seal_message,
  verify_envelope,
} from "./messages.js";
import type { PrivateStore } from "./store.js";
import type { RelayTransport } from "./transport.js";

export interface PriceQuote {
  readonly rate: string;
  readonly observed_at: number;
  readonly valid_until: number;
}
export interface MakerRuntimeConfig {
  domain: RfqDomain;
  core: ProtocolCore;
  identity: MessagingIdentity;
  store: PrivateStore;
  relay: RelayTransport;
  preflight(): Promise<void>;
  price(request: Readonly<IntentInput>): Promise<PriceQuote>;
  capacity(): Promise<{ ZEC: string; USDC: string; ETH: string }>;
  fees(request: Readonly<IntentInput>): Promise<readonly FeeQuote[]>;
  receiving(): Promise<ReceivingDescriptor>;
}
const response_record = z.strictObject({
  request_digest: z.string(),
  response: envelope,
  offer_id: z.string().nullable(),
  deadline: z.number().int(),
});
export class RfqMaker {
  readonly domain: RfqDomain;
  private options: MakerStartOptions | undefined;
  private starting = false;
  private controller: AbortController | undefined;
  private loop: Promise<void> | undefined;
  private state: "stopped" | "running" | "paused" = "stopped";
  private blockers: string[] = [];
  private announcement: MakerAnnouncement | undefined;
  private readonly live = new Map<string, number>();
  constructor(private readonly config: MakerRuntimeConfig) {
    this.domain = rfq_domain.parse(config.domain);
  }
  private validate_options(value: MakerStartOptions): MakerStartOptions {
    const options = maker_start_options.parse(value);
    for (const pair of options.pairs) {
      const [from = "", to = ""] = pair.split("/");
      require_pair(this.domain.policy, from, to);
    }
    for (const asset of ["ZEC", "USDC"] as const) {
      const range = options.limits.per_trade[asset];
      this.config.core.amount(asset, range.min);
      this.config.core.amount(asset, range.max);
      this.config.core.amount(asset, options.limits.max_committed[asset], false);
      if (this.config.core.compare(asset, range.min, range.max) > 0)
        throw new ZuttiError("invalid_input", "Maker trade limits are reversed");
    }
    for (const asset of ["ZEC", "USDC", "ETH"] as const)
      this.config.core.amount(asset, options.limits.max_fee_per_swap[asset], false);
    return options;
  }
  async start(value: MakerStartOptions): Promise<void> {
    if (this.state !== "stopped" || this.starting)
      throw new ZuttiError("invalid_input", "Maker service is already started");
    this.starting = true;
    const controller = new AbortController();
    this.controller = controller;
    try {
      const options = this.validate_options(value);
      await this.provider(this.config.preflight(), controller.signal);
      const capacity = await this.provider(this.config.capacity(), controller.signal);
      for (const asset of ["ZEC", "USDC", "ETH"] as const)
        this.config.core.amount(asset, capacity[asset], false);
      for (const id of await this.config.store.list("maker-response-")) {
        if (controller.signal.aborted)
          throw new ZuttiError("aborted", "Maker startup was interrupted");
        const record = response_record.parse(await this.config.store.read(id));
        if (record.offer_id && record.deadline > Date.now())
          this.live.set(record.offer_id, record.deadline);
      }
      if (controller.signal.aborted)
        throw new ZuttiError("aborted", "Maker startup was interrupted");
      this.options = options;
      await this.advertise();
      if (controller.signal.aborted)
        throw new ZuttiError("aborted", "Maker startup was interrupted");
      this.state = "running";
      this.loop = this.work(controller.signal);
    } finally {
      this.starting = false;
    }
  }
  private async provider<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
    if (signal.aborted) throw new ZuttiError("aborted", "Maker operation was interrupted");
    let timer: ReturnType<typeof setTimeout> | undefined;
    let abort: (() => void) | undefined;
    const interrupted = new Promise<never>((_resolve, reject) => {
      abort = () => reject(new ZuttiError("aborted", "Maker operation was interrupted"));
      signal.addEventListener("abort", abort, { once: true });
      timer = setTimeout(
        () => reject(new ZuttiError("service_unavailable", "Maker provider timed out")),
        2_000,
      );
    });
    try {
      return await Promise.race([promise, interrupted]);
    } finally {
      if (timer) clearTimeout(timer);
      if (abort) signal.removeEventListener("abort", abort);
    }
  }
  private async advertise(): Promise<void> {
    if (!this.options) throw new ZuttiError("invalid_input", "Maker service has no configuration");
    this.announcement = await announce(this.config.identity, {
      network: this.domain.network,
      deployment_id: this.domain.deployment_id,
      key_version: 1,
      pairs: this.options.pairs,
      policy: this.domain.policy,
      deadline: Date.now() + 300_000,
    });
    await this.config.relay.announce(this.announcement);
  }
  async pause(): Promise<void> {
    if (this.state !== "running") throw new ZuttiError("invalid_input", "Maker is not running");
    this.state = "paused";
  }
  async resume(): Promise<void> {
    if (this.state !== "paused") throw new ZuttiError("invalid_input", "Maker is not paused");
    await this.config.preflight();
    this.state = "running";
  }
  async stop(): Promise<{ state: "stopped"; pending_swap_ids: readonly string[] }> {
    this.state = "stopped";
    this.controller?.abort();
    await this.loop;
    this.controller = undefined;
    this.loop = undefined;
    return { state: "stopped", pending_swap_ids: [] };
  }
  async summary() {
    const now = Date.now();
    for (const [id, deadline] of this.live) if (deadline <= now) this.live.delete(id);
    const balances = await this.config.capacity();
    return {
      maker_id: this.config.identity.id,
      state: this.state,
      balance: Object.fromEntries(
        Object.entries(balances).map(([asset, amount]) => [asset, { available: true, amount }]),
      ),
      reserved: { ZEC: { available: true, amount: "0" }, USDC: { available: true, amount: "0" } },
      active_swaps: 0,
      live_offers: this.live.size,
      blockers: [...this.blockers],
      checked_at: now,
    };
  }
  private async work(signal: AbortSignal): Promise<void> {
    while (!signal.aborted) {
      if (this.state === "running") {
        try {
          if (!this.announcement || this.announcement.deadline - Date.now() < 60_000)
            await this.advertise();
          for (const value of await this.config.relay.poll(
            await authorize_poll(this.config.identity, this.domain, Date.now()),
            signal,
          )) {
            try {
              await this.respond(value, signal);
            } catch (error) {
              if (
                error instanceof ZuttiError &&
                ["persistence_failure", "pricing_unavailable"].includes(error.code)
              )
                throw error;
            }
          }
          this.blockers = [];
        } catch (error) {
          if (!signal.aborted)
            this.blockers = [error instanceof ZuttiError ? error.code : "service_unavailable"];
        }
      }
      if (!signal.aborted)
        await new Promise((resolve) => setTimeout(resolve, this.blockers.length ? 500 : 100));
    }
  }
  private async respond(value: unknown, signal: AbortSignal): Promise<void> {
    const message = await verify_envelope(value, this.domain, Date.now());
    if (message.type !== "request")
      throw new ZuttiError("invalid_message", "Expected an RFQ request");
    const payload = request_body.parse(
      await open_message(this.config.identity, message, this.domain, Date.now()),
    );
    if (
      payload.request_id !== message.request_id ||
      payload.deadline !== message.deadline ||
      canonical(payload.policy) !== canonical(this.domain.policy)
    )
      throw new ZuttiError("invalid_message", "Request binding does not match");
    const id = `maker-response-${message.request_id}-${message.sender_id}`;
    const request_digest = await digest("request-envelope", message);
    let stored = await this.config.store.read(id);
    if (stored === undefined) {
      const response = await this.make_response(message, payload, signal);
      await this.config.store.create(id, {
        request_digest,
        response: response.message,
        offer_id: response.offer_id,
        deadline: response.message.deadline,
      });
      stored = await this.config.store.read(id);
    }
    const record = response_record.parse(stored);
    if (record.request_digest !== request_digest)
      throw new ZuttiError("replay", "Request identifier was reused for different content");
    if (record.deadline <= Date.now()) return;
    if (record.offer_id) this.live.set(record.offer_id, record.deadline);
    await this.config.relay.send(record.response, signal);
  }
  private async make_response(
    message: z.infer<typeof envelope>,
    payload: z.infer<typeof request_body>,
    signal: AbortSignal,
  ) {
    const options = this.options;
    if (!options || signal.aborted || this.state !== "running")
      throw new ZuttiError("service_unavailable", "Maker is not admitting requests");
    const request = payload.request;
    const range = options.limits.per_trade[request.from];
    this.config.core.amount(request.from, request.amount);
    let reason: "unsupported" | "limits" | "unavailable" | undefined;
    if (!options.pairs.includes(`${request.from}/${request.to}` as "ZEC/USDC" | "USDC/ZEC"))
      reason = "unsupported";
    else if (
      this.config.core.compare(request.from, request.amount, range.min) < 0 ||
      this.config.core.compare(request.from, request.amount, range.max) > 0
    )
      reason = "limits";
    if (reason) {
      const response = await seal_message(
        this.config.identity,
        {
          network: this.domain.network,
          deployment_id: this.domain.deployment_id,
          type: "decline",
          request_id: message.request_id,
          recipient_id: message.sender_id,
          recipient_key: payload.reply_key,
          deadline: payload.deadline,
        },
        { request_id: message.request_id, reason },
      );
      return { message: response, offer_id: null };
    }
    const price = await this.provider(this.config.price(Object.freeze({ ...request })), signal);
    const now = Date.now();
    if (
      !Number.isSafeInteger(price.observed_at) ||
      !Number.isSafeInteger(price.valid_until) ||
      price.observed_at > now ||
      now - price.observed_at > 30_000 ||
      price.valid_until <= now
    )
      throw new ZuttiError("pricing_unavailable", "Maker price is unavailable or stale");
    let amounts;
    try {
      amounts = this.config.core.quote(request, price.rate, options.fee_bps);
    } catch {
      throw new ZuttiError("pricing_unavailable", "Maker price cannot produce a valid quote");
    }
    const capacity = await this.provider(this.config.capacity(), signal);
    if (
      this.config.core.compare(
        request.to,
        amounts.net,
        this.config.core.amount(request.to, capacity[request.to], false).amount,
      ) > 0 ||
      this.config.core.compare(request.to, amounts.net, options.limits.max_committed[request.to]) >
        0
    )
      throw new ZuttiError("pricing_unavailable", "Maker inventory cannot cover this quote");
    const fees = [
      ...(await this.provider(this.config.fees(Object.freeze({ ...request })), signal)),
    ];
    for (const fee of fees)
      if (
        fee.paid_by === "maker" &&
        (this.config.core.compare(
          fee.asset,
          fee.max_amount,
          options.limits.max_fee_per_swap[fee.asset],
        ) > 0 ||
          this.config.core.compare(fee.asset, fee.max_amount, capacity[fee.asset]) > 0)
      )
        throw new ZuttiError("pricing_unavailable", "Maker fee capacity is insufficient");
    if (signal.aborted || this.state !== "running")
      throw new ZuttiError("aborted", "Maker quote was interrupted");
    const deadline = Math.min(
      payload.deadline,
      now + Number.parseInt(options.quote_deadline, 10) * 1000,
      price.valid_until,
    );
    const body: OfferBody = {
      intent_id: payload.intent_id,
      request_id: payload.request_id,
      maker_id: this.config.identity.id,
      receiving: await this.provider(this.config.receiving(), signal),
      input: { asset: request.from, amount: amounts.input },
      gross_output: { asset: request.to, amount: amounts.gross },
      maker_fee: { bps: options.fee_bps, amount: { asset: request.to, amount: amounts.fee } },
      output: { asset: request.to, amount: amounts.net },
      fees,
      deadline,
      policy: this.domain.policy,
      nonce: random_id(),
    };
    const offer_id = await digest("offer", body);
    try {
      await verify_offer(
        { ...body, id: offer_id },
        request,
        payload.intent_id,
        payload.request_id,
        this.config.identity.id,
        this.domain,
        this.config.core,
        Date.now(),
        payload.deadline,
      );
    } catch {
      throw new ZuttiError("pricing_unavailable", "Maker quote or fees do not meet the request");
    }
    const response = await seal_message(
      this.config.identity,
      {
        network: this.domain.network,
        deployment_id: this.domain.deployment_id,
        type: "offer",
        request_id: message.request_id,
        recipient_id: message.sender_id,
        recipient_key: payload.reply_key,
        deadline,
      },
      { ...body, id: offer_id },
    );
    return { message: response, offer_id };
  }
}
