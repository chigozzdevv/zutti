import { readFile } from "node:fs/promises";
import { rfq_domain } from "@zutti/schema";
import { create_relay_server } from "./app.js";

const profile = process.env.ZUTTI_RFQ_PROFILE;
if (!profile) throw new Error("ZUTTI_RFQ_PROFILE must identify a reviewed public RFQ profile");
const domain = rfq_domain.parse(JSON.parse(await readFile(profile, "utf8")) as unknown);
const port = Number(process.env.PORT ?? "3001");
if (!Number.isInteger(port) || port < 1 || port > 65_535) throw new Error("Invalid PORT");
const server = create_relay_server(domain);
server.listen(port, "127.0.0.1");
for (const signal of ["SIGTERM", "SIGINT"] as const)
  process.on(signal, () => {
    server.close();
    server.closeIdleConnections();
  });
