// Local signed build: reads the updater key and password from ~/.deadlock-mm.
// Usage: pnpm build:local [nsis,msi]
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const keyDir = join(homedir(), ".deadlock-mm");
const env = {
  ...process.env,
  TAURI_SIGNING_PRIVATE_KEY: join(keyDir, "updater.key"),
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD: readFileSync(join(keyDir, "updater.password"), "utf8").trim(),
};
const bundles = process.argv[2] ?? "nsis";
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const r = spawnSync("pnpm", ["tauri", "build", "--bundles", bundles], { cwd: root, env, stdio: "inherit", shell: true });
process.exit(r.status ?? 1);
