import { ZuttiError } from "@zutti/schema";

export function canonical(value: unknown): string {
  if (value === null || typeof value === "string" || typeof value === "boolean")
    return JSON.stringify(value);
  if (typeof value === "number" && Number.isSafeInteger(value)) return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (
    typeof value === "object" &&
    value !== null &&
    Object.getPrototypeOf(value) === Object.prototype
  ) {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(record[key])}`)
      .join(",")}}`;
  }
  throw new ZuttiError("invalid_message", "Message contains unsupported values");
}
export function hex(bytes: ArrayBuffer | Uint8Array): string {
  return Array.from(bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}
export function unhex(value: string): Uint8Array<ArrayBuffer> {
  if (value.length % 2 !== 0 || !/^[0-9a-f]*$/.test(value))
    throw new ZuttiError("invalid_message", "Invalid byte encoding");
  return Uint8Array.from(value.match(/../g) ?? [], (byte) => Number.parseInt(byte, 16));
}
export const text = (value: string): Uint8Array<ArrayBuffer> => new TextEncoder().encode(value);
export const random_id = (): string => hex(crypto.getRandomValues(new Uint8Array(32)));
export async function digest(purpose: string, value: unknown): Promise<string> {
  return hex(
    await crypto.subtle.digest(
      "SHA-256",
      text(canonical({ protocol: "zutti-rfq", version: 1, purpose, value })),
    ),
  );
}
export function immutable<T>(value: T): Readonly<T> {
  if (value && typeof value === "object") {
    Object.freeze(value);
    for (const child of Object.values(value)) immutable(child);
  }
  return value;
}
