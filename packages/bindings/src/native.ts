import { execFileSync } from "node:child_process";
import { isAbsolute } from "node:path";
import { ZuttiError, amount as amount_schema } from "@zutti/schema";
import type { ProtocolCore, AmountResult, QuoteResult } from "./core.js";

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new ZuttiError("service_unavailable", "Invalid native core response");
  return value as Record<string, unknown>;
}
export function create_native_core(binary: string): ProtocolCore {
  if (!isAbsolute(binary))
    throw new ZuttiError("invalid_input", "The native core executable must use an absolute path");
  function invoke(request: Record<string, unknown>): Record<string, unknown> {
    let raw: unknown;
    try {
      raw = JSON.parse(
        execFileSync(binary, {
          input: JSON.stringify(request),
          encoding: "utf8",
          timeout: 5_000,
          maxBuffer: 16_384,
          stdio: ["pipe", "pipe", "ignore"],
        }),
      );
    } catch {
      throw new ZuttiError("service_unavailable", "Native protocol core is unavailable");
    }
    const response = object(raw);
    if (response.version !== 1 || typeof response.ok !== "boolean")
      throw new ZuttiError("service_unavailable", "Unsupported native core response");
    if (!response.ok)
      throw new ZuttiError(
        "invalid_input",
        "Protocol input is invalid or outside its supported range",
      );
    return object(response.result);
  }
  function quote(request: Record<string, unknown>): QuoteResult {
    const result = invoke(request);
    return {
      input: amount_schema.parse(result.input),
      gross: amount_schema.parse(result.gross),
      net: amount_schema.parse(result.net),
      fee: amount_schema.parse(result.fee),
    };
  }
  return Object.freeze({
    amount(asset, amount, positive = true): AmountResult {
      const result = invoke({ method: "amount", asset, amount, positive });
      if (typeof result.units !== "string" || !/^(0|[1-9][0-9]*)$/.test(result.units))
        throw new ZuttiError("service_unavailable", "Invalid native units");
      return { amount: amount_schema.parse(result.amount), units: result.units };
    },
    compare(asset, left, right) {
      const ordering = invoke({ method: "compare", asset, left, right }).ordering;
      if (ordering !== -1 && ordering !== 0 && ordering !== 1)
        throw new ZuttiError("service_unavailable", "Invalid native comparison");
      return ordering;
    },
    net(asset, gross, fee_bps) {
      return quote({ method: "net", asset, gross, fee_bps });
    },
    quote(input, rate, fee_bps) {
      return quote({
        method: "quote",
        from: input.from,
        to: input.to,
        amount: input.amount,
        rate,
        fee_bps,
      });
    },
  } satisfies ProtocolCore);
}
