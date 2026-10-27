import { readFile, writeFile, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { format, resolveConfig } from "prettier";

const root = new URL("../../", import.meta.url);
const contracts = {
  PrivatePool: "private-pool",
  SourceRegistry: "source-registry",
  PreparationVerifier: "preparation-verifier",
  SpvPool: "spv-pool",
  SpvRegistry: "spv-registry",
};
const destination = new URL("src/generated/", import.meta.url);
const formatting = await resolveConfig(fileURLToPath(root));
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
    await format(`export const abi = ${JSON.stringify(artifact.abi, null, 2)} as const;\n`, {
      ...formatting,
      parser: "typescript",
    }),
  );
}
await writeFile(
  new URL("index.ts", destination),
  Object.values(contracts)
    .map((name) => `export { abi as ${name.replaceAll("-", "_")}_abi } from "./${name}.js";`)
    .join("\n") + "\n",
);
process.stdout.write(`Generated contract ABIs in ${fileURLToPath(destination)}\n`);
