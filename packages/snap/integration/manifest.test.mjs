import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { getSnapChecksum, VirtualFile, SnapManifestStruct } from "@metamask/snaps-utils";

test("the installable manifest pins its files and requests only the required permissions", async () => {
  const manifest = JSON.parse(
    await readFile(new URL("../dist/snap.manifest.json", import.meta.url), "utf8"),
  );
  SnapManifestStruct.assert(manifest);
  assert.deepEqual(Object.keys(manifest.initialPermissions).sort(), [
    "endowment:rpc",
    "endowment:webassembly",
    "snap_dialog",
    "snap_getEntropy",
  ]);
  assert.deepEqual(manifest.source.files, ["./files/wallet.wasm"]);
  assert.equal(manifest.source.location.npm.filePath, "bundle.js");
  const checksum = await getSnapChecksum({
    manifest: new VirtualFile({ path: "snap.manifest.json", value: "", result: manifest }),
    sourceCode: new VirtualFile({
      path: "bundle.js",
      value: await readFile(new URL("../dist/bundle.js", import.meta.url)),
    }),
    auxiliaryFiles: [
      new VirtualFile({
        path: "files/wallet.wasm",
        value: await readFile(new URL("../dist/files/wallet.wasm", import.meta.url)),
      }),
    ],
    localizationFiles: [],
  });
  assert.equal(manifest.source.shasum, checksum);
});
