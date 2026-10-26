/**
 * E2E test configuration using WebdriverIO + tauri-plugin-webdriver.
 *
 * The plugin runs a W3C WebDriver server inside the app on port 4445.
 * Works on macOS, Linux, and Windows — no external tauri-driver needed.
 *
 * Prerequisites:
 *   1. Build debug binary: `cargo build --manifest-path src-tauri/Cargo.toml`
 *   2. Run tests: `bun run test:e2e`
 *      (Vite dev server is started automatically by the test runner)
 */

import type { } from "@wdio/types";
import { spawn, type ChildProcess } from "child_process";
import fs from "fs";
import net from "net";
import os from "os";
import path from "path";

const ROOT = path.resolve(__dirname, "../..");

const APP_BINARY =
  process.env.ROWEL_APP_BINARY ??
  path.join(ROOT, "src-tauri", "target", "debug", "rowel");

const VITE_PORT = process.env.E2E_VITE_PORT ?? "1420";

// Named in the environment so the spec workers, which load this file again
// in processes of their own, find the same directory the launcher made: the
// agent helper reaches the app's broker socket in it (helpers/agent.ts).
const TEST_DB_DIR = (process.env.ROWEL_E2E_DB_DIR ??= path.join(
  os.tmpdir(),
  `rowel-e2e-${Date.now()}`,
));

const SPECS_DIR = path.join(__dirname, "specs");

/**
 * The spec files this run executes.
 *
 * Specs run one at a time (maxInstances: 1) because the in-app WebDriver
 * server takes a single session, so CI splits the suite across parallel jobs
 * instead: with `E2E_SHARD=k/n` this run takes every n-th spec starting at
 * the k-th (1-based), over the alphabetically sorted list. Every shard sees
 * the same order, so the shards are disjoint and together cover every spec.
 * Unset (local runs) means all of them.
 */
function shardedSpecs(): string[] {
  const all = fs
    .readdirSync(SPECS_DIR)
    .filter((f) => f.endsWith(".spec.ts"))
    .sort()
    .map((f) => path.join(SPECS_DIR, f));

  const shard = process.env.E2E_SHARD;
  if (!shard) return all;

  const match = /^(\d+)\/(\d+)$/.exec(shard);
  const index = match ? Number(match[1]) : NaN;
  const total = match ? Number(match[2]) : NaN;
  if (!match || total < 1 || index < 1 || index > total) {
    throw new Error(`[e2e] E2E_SHARD must look like "1/3" (1-based), got "${shard}"`);
  }
  const mine = all.filter((_, i) => i % total === index - 1);
  console.log(`[e2e] shard ${index}/${total}: ${mine.length} of ${all.length} specs`);
  return mine;
}

let viteProcess: ChildProcess;
let appProcess: ChildProcess;

/** Poll via HTTP GET until a server responds or we time out. */
async function waitForHttp(url: string, timeoutMs: number, label: string): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      await fetch(url);
      return;
    } catch {
      await new Promise<void>((r) => setTimeout(r, 300));
    }
  }
  throw new Error(`[e2e] ${label} did not respond at ${url} within ${timeoutMs}ms`);
}

/** Poll via TCP until a port accepts connections, the process exits, or we time out. */
async function waitForPort(
  port: number,
  timeoutMs: number,
  label: string,
  process?: ChildProcess,
): Promise<void> {
  let exitCode: number | null = null;
  process?.once("exit", (code) => { exitCode = code ?? 1; });

  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (exitCode !== null) {
      throw new Error(`[e2e] ${label} process exited with code ${exitCode} before port ${port} opened`);
    }
    if (await portOpen(port)) return;
    await new Promise<void>((r) => setTimeout(r, 250));
  }
  throw new Error(`[e2e] ${label} did not open port ${port} within ${timeoutMs}ms`);
}

/** One TCP connect attempt: is something listening on `port` right now? */
function portOpen(port: number): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    const s = net.connect(port, "127.0.0.1");
    s.once("connect", () => { s.destroy(); resolve(true); });
    s.once("error", () => { s.destroy(); resolve(false); });
  });
}

export const config: WebdriverIO.Config = {
  hostname: "127.0.0.1",
  port: 4445,
  path: "/",

  specs: shardedSpecs(),
  maxInstances: 1,

  capabilities: [
    {
      maxInstances: 1,
      browserName: "chrome",
      "wdio:enforceWebDriverClassic": true,
    } as WebdriverIO.Capabilities,
  ],

  framework: "mocha",
  reporters: ["spec"],

  mochaOpts: {
    ui: "bdd",
    timeout: 60_000,
  },

  // WebdriverIO v9 dropped `autoCompileOpts`/ts-node and transpiles specs with
  // tsx automatically; `tsConfigPath` is how you point it at our tsconfig.
  tsConfigPath: path.join(__dirname, "tsconfig.json"),

  // No global `before` hook: waiting for the setup screen here would assume
  // every spec inherits a pristine vault, which is exactly the run-order
  // coupling this harness now avoids. Each spec opens with its own
  // `resetPristine()` / `resetEmpty()` (see helpers/reset.ts).

  onPrepare: async () => {
    fs.mkdirSync(TEST_DB_DIR, { recursive: true });

    // Start the Vite dev server — the debug binary loads the frontend from
    // http://localhost:1420 (baked in at compile time by tauri-build; a binary
    // built with another `build.devUrl` needs E2E_VITE_PORT to match). Strict:
    // if the port is taken, fail here rather than let Vite drift to the next
    // one while the app, and this readiness probe, talk to whatever holds it.
    console.log("[e2e] Starting Vite dev server...");
    // Invoke the vite binary directly — avoids relying on `bun` being in PATH
    // when wdio is running under Node.js.
    const viteBin = path.join(ROOT, "node_modules", ".bin", "vite");
    viteProcess = spawn(viteBin, ["--port", VITE_PORT, "--strictPort"], {
      cwd: ROOT,
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env },
    });
    viteProcess.stdout?.on("data", (d: Buffer) =>
      process.stdout.write(`[vite] ${d}`),
    );
    viteProcess.stderr?.on("data", (d: Buffer) =>
      process.stderr.write(`[vite] ${d}`),
    );
    viteProcess.on("exit", (code) => {
      if (code !== null && code !== 0) {
        console.error(`[vite] exited with code ${code}`);
      }
    });
    // Use HTTP polling — avoids IPv4/IPv6 mismatch that TCP connect can hit
    await waitForHttp(`http://localhost:${VITE_PORT}`, 30_000, "Vite");
    console.log(`[e2e] Vite ready on :${VITE_PORT}`);

    // Start the app binary — it loads the frontend from Vite and starts
    // the WebDriver server on :4445. That port must be ours to open: another
    // debug Rowel already running (a second checkout's dev app, say) holds
    // it, the single-instance plugin quietly exits the binary we spawn, and
    // the whole suite then drives the wrong app. Seen locally; every reset
    // was refused by the ROWEL_E2E gate, but nothing else would have been.
    if (await portOpen(4445)) {
      throw new Error(
        "[e2e] port 4445 is already in use — is another Rowel instance running? Quit it first.",
      );
    }
    console.log(`[e2e] Starting app (${APP_BINARY})`);
    appProcess = spawn(APP_BINARY, [], {
      cwd: ROOT,
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...process.env,
        // Both are gates on the `e2e_reset` command: it refuses to run unless
        // ROWEL_E2E is 1, and it only ever wipes ROWEL_DB_DIR — so it can
        // never reach a developer's real app-data dir. See commands/e2e.rs.
        ROWEL_DB_DIR: TEST_DB_DIR,
        ROWEL_E2E: "1",
        WEBKIT_DISABLE_COMPOSITING_MODE: "1",
      },
    });
    appProcess.stdout?.on("data", (d: Buffer) =>
      process.stdout.write(`[app] ${d}`),
    );
    appProcess.stderr?.on("data", (d: Buffer) =>
      process.stderr.write(`[app] ${d}`),
    );
    appProcess.on("exit", (code) => {
      if (code !== null && code !== 0) {
        console.error(`[app] exited with code ${code}`);
      }
    });

    await waitForPort(4445, 30_000, "App WebDriver server", appProcess);
    console.log("[e2e] App WebDriver server ready on :4445");
  },

  onComplete: () => {
    appProcess?.kill();
    viteProcess?.kill();
    try {
      fs.rmSync(TEST_DB_DIR, { recursive: true, force: true });
    } catch {
      // ignore
    }
  },
};
