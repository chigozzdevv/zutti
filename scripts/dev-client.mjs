import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const build = spawn("pnpm", ["build:ts"], { cwd: root, stdio: "inherit" });
const result = await new Promise((resolve) => build.on("exit", resolve));
if (result !== 0) process.exit(result ?? 1);
const children = [
  spawn(process.execPath, ["server/dist/ui-server.js"], { cwd: root, stdio: "inherit" }),
  spawn("pnpm", ["--filter", "@zutti/client", "dev"], { cwd: root, stdio: "inherit" }),
];
let closing = false;
const close = () => {
  if (closing) return;
  closing = true;
  for (const child of children) child.kill("SIGTERM");
};
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, close);
for (const child of children) {
  child.on("error", (error) => {
    console.error(error.message);
    process.exitCode = 1;
    close();
  });
  child.on("exit", (code) => {
    if (!closing) {
      process.exitCode = code ?? 1;
      close();
    }
  });
}
