import { cp, mkdir, writeFile, chmod } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { build } from "../client/node_modules/vite/dist/node/index.js";

const root = fileURLToPath(new URL("../", import.meta.url));
const destination = `${root}.release`;
await build({
  configFile: false,
  root,
  ssr: { noExternal: true },
  build: {
    ssr: `${root}server/dist/ui-server.js`,
    target: "node22",
    outDir: `${destination}/server/dist`,
    emptyOutDir: true,
    rolldownOptions: { output: { entryFileNames: "ui-server.js" } },
  },
});
await cp(`${root}client/dist`, `${destination}/client/dist`, { recursive: true });
for (const name of [
  "optimized/sepolia.json",
  "optimized/source-profile.json",
  "optimized/swap-proof.json",
  "swap-proof.json",
]) {
  const target = `${destination}/deploy/ethereum/${name}`;
  await mkdir(target.slice(0, target.lastIndexOf("/")), { recursive: true });
  await cp(`${root}deploy/ethereum/${name}`, target);
  await chmod(target, 0o644);
}
await writeFile(
  `${destination}/package.json`,
  JSON.stringify({ name: "zutti-app", private: true, type: "module" }),
);
console.log(
  "Packaged the app and public deployment records; no wallet or prover credentials included.",
);
