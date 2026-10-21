import { readFile, writeFile, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const root = new URL("../../", import.meta.url);
const contracts = {
  PrivatePool: "private-pool",
  SourceRegistry: "source-registry",
  PreparationVerifier: "preparation-verifier",
};
const destination = new URL("src/generated/", import.meta.url);
await mkdir(destination, { recursive: true });
for (const [contract, file] of Object.entries(contracts)) {
  const artifact: unknown = JSON.parse(
    await readFile(new URL(`contracts/out/${contract}.sol/${contract}.json`, root), "utf8"),
  );
  if (
    !artifact ||
    typeof artifact !== "object" ||
    !("abi" in artifact) ||
    !Array.isArray(artifact.abi)
  )
    throw new Error(`Missing ABI: ${contract}`);
  await writeFile(
    new URL(`${file}.ts`, destination),
    `export const abi = ${JSON.stringify(artifact.abi, null, 2)} as const;\n`,
  );
}
await writeFile(
  new URL("index.ts", destination),
  Object.values(contracts)
    .map((name) => `export { abi as ${name.replaceAll("-", "_")}_abi } from "./${name}.js";`)
    .join("\n") + "\n",
);
process.stdout.write(`Generated contract ABIs in ${fileURLToPath(destination)}\n`);
