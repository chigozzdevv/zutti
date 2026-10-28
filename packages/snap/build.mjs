import { mkdir, readFile, writeFile, copyFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { build } from "../../client/node_modules/vite/dist/node/index.js";
import { getSnapChecksum, VirtualFile, SnapManifestStruct } from "@metamask/snaps-utils";
import { allowed_origins } from "./src/policy.ts";

const root = fileURLToPath(new URL("./", import.meta.url));
await mkdir(`${root}dist/files`, { recursive: true });
await copyFile(`${root}generated/wallet_bg.wasm`, `${root}dist/files/wallet.wasm`);
await build({
  configFile: false,
  root,
  define: { "process.env.NODE_ENV": '"production"' },
  build: {
    emptyOutDir: false,
    target: "es2022",
    minify: false,
    lib: { entry: `${root}src/index.ts`, formats: ["cjs"], fileName: () => "bundle.js" },
    rolldownOptions: { output: { codeSplitting: false } },
  },
});
const bundle = await readFile(`${root}dist/bundle.js`);
const wasm = await readFile(`${root}dist/files/wallet.wasm`);
const manifest = {
  version: "0.1.0",
  description: "Zcash Testnet payment signing with independent Zutti reservation verification",
  proposedName: "Zutti Testnet",
  repository: { type: "git", url: "https://github.com/chigozzdevv/zutti.git" },
  source: {
    shasum: "",
    location: {
      npm: {
        filePath: "bundle.js",
        packageName: "@zutti/snap",
        registry: "https://registry.npmjs.org/",
      },
    },
    files: ["./files/wallet.wasm"],
  },
  initialPermissions: {
    snap_dialog: {},
    snap_getEntropy: {},
    "endowment:webassembly": {},
    "endowment:rpc": { allowedOrigins: allowed_origins, maxRequestTime: 180000 },
  },
  manifestVersion: "0.1",
};
manifest.source.shasum = await getSnapChecksum({
  manifest: new VirtualFile({ path: "snap.manifest.json", value: "", result: manifest }),
  sourceCode: new VirtualFile({ path: "bundle.js", value: bundle }),
  auxiliaryFiles: [new VirtualFile({ path: "files/wallet.wasm", value: wasm })],
  localizationFiles: [],
});
SnapManifestStruct.assert(manifest);
await writeFile(`${root}dist/snap.manifest.json`, `${JSON.stringify(manifest, null, 2)}\n`);
