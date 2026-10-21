import test from "node:test";
import assert from "node:assert/strict";
import { create_native_core } from "../dist/native.js";

const binary = process.env.ZUTTI_CORE_BINARY;
if (!binary) throw new Error("ZUTTI_CORE_BINARY must select the compiled native core");
const core = create_native_core(binary);
test("Rust owns amount precision, range and normalization", () => {
  assert.deepEqual(core.amount("ZEC", "5.00000000"), { amount: "5", units: "500000000" });
  assert.deepEqual(core.amount("USDC", "5000"), { amount: "5000", units: "5000000000" });
  for (const [asset, amount] of [
    ["ZEC", "0"],
    ["ZEC", "21000001"],
    ["USDC", "1.0000001"],
    ["ETH", "1e3"],
  ])
    assert.throws(() => core.amount(asset, amount));
  assert.deepEqual(core.amount("USDC", "0", false), { amount: "0", units: "0" });
});
test("both quote directions use exact native arithmetic and output fees", () => {
  assert.deepEqual(core.quote({ from: "ZEC", to: "USDC", amount: "5" }, "40", 50), {
    input: "5",
    gross: "200",
    net: "199",
    fee: "1",
  });
  assert.deepEqual(core.quote({ from: "USDC", to: "ZEC", amount: "100" }, "40", 50), {
    input: "100",
    gross: "2.5",
    net: "2.4875",
    fee: "0.0125",
  });
  assert.equal(core.compare("ZEC", "5.00000000", "5"), 0);
  assert.throws(() => core.net("USDC", "0.000001", 9999));
});
