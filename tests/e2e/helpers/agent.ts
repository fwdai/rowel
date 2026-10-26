import net from "net";
import path from "path";

/**
 * An agent client for the grant dialog's spec: the extension protocol's
 * sealed channel, spoken over the app's broker socket the way `rowel mcp`
 * speaks it, so a spec can raise an `associate` or a grant ask in the app
 * under test and read the reply its dialog settles.
 *
 * In Node rather than through the built `rowel` binary because the binary
 * keeps its client key in the OS secure store on macOS, where a test must
 * not reach, and because a spec needs a request without a run id, which
 * `rowel` never sends. The crypto is the extension's own tweetnacl
 * (`extension/rowel/background/nacl.min.js`), so this speaks exactly what the
 * extension does: native messaging frames (a native-endian u32 length, then
 * JSON), `change-public-keys` in the clear, then every action sealed with
 * NaCl's box under a fresh nonce, the reply under that nonce plus one.
 */

// eslint-disable-next-line @typescript-eslint/no-require-imports
const nacl = require(path.resolve(__dirname, "../../../extension/rowel/background/nacl.min.js"));

const CLIENT_ID = "rowel-e2e";
const PUSHED = ["rowel-signal", "database-locked"];

type Json = Record<string, unknown>;

/** A refusal the broker sent in the clear: its `errorCode` and text. */
export class Refused extends Error {
  constructor(readonly code: number, message: string) {
    super(`refused ${code}: ${message}`);
  }
}

/** The socket of the app the suite drives, in its throwaway data directory. */
export function brokerSocket(): string {
  const dir = process.env.ROWEL_E2E_DB_DIR;
  if (!dir) throw new Error("[e2e] ROWEL_E2E_DB_DIR is not set — run through wdio.conf.ts");
  return path.join(dir, "browser.sock");
}

const b64 = (bytes: Uint8Array) => Buffer.from(bytes).toString("base64");
const unb64 = (text: string) => new Uint8Array(Buffer.from(text, "base64"));

// The protocol's nonce increment: little-endian, carrying from the first byte.
function next(nonce: Uint8Array): Uint8Array {
  const out = Uint8Array.from(nonce);
  for (let i = 0, carry = 1; i < out.length && carry; i++) {
    const sum = out[i] + carry;
    out[i] = sum & 0xff;
    carry = sum >> 8;
  }
  return out;
}

export class Agent {
  private readonly keys = nacl.box.keyPair();
  private readonly idKey = b64(nacl.box.keyPair().publicKey);
  private hostKey?: Uint8Array;
  private buffer = Buffer.alloc(0);
  private waiting: ((frame: Json) => void)[] = [];

  private constructor(private readonly socket: net.Socket) {
    socket.on("data", (chunk: Buffer) => {
      this.buffer = Buffer.concat([this.buffer, chunk]);
      while (this.buffer.length >= 4) {
        const length = this.buffer.readUInt32LE(0);
        if (this.buffer.length < 4 + length) break;
        const frame = JSON.parse(this.buffer.subarray(4, 4 + length).toString("utf8")) as Json;
        this.buffer = this.buffer.subarray(4 + length);
        // What the broker pushes unasked, to every connection: a wake-up for
        // the extensions and the lock notice. Neither is a reply.
        if (PUSHED.includes(String(frame.action))) continue;
        this.waiting.shift()?.(frame);
      }
    });
  }

  /** Connect and exchange keys. */
  static async connect(socketPath = brokerSocket()): Promise<Agent> {
    const socket = await new Promise<net.Socket>((resolve, reject) => {
      const s = net.connect(socketPath, () => resolve(s));
      s.once("error", reject);
    });
    const agent = new Agent(socket);
    const nonce = nacl.randomBytes(24);
    const reply = await agent.roundTrip({
      action: "change-public-keys",
      publicKey: b64(agent.keys.publicKey),
      nonce: b64(nonce),
      clientID: CLIENT_ID,
    });
    if (reply.success !== "true" || reply.nonce !== b64(next(nonce))) {
      throw new Error(`[e2e] the key exchange failed: ${JSON.stringify(reply)}`);
    }
    agent.hostKey = unb64(String(reply.publicKey));
    return agent;
  }

  private roundTrip(request: Json): Promise<Json> {
    const body = Buffer.from(JSON.stringify(request), "utf8");
    const length = Buffer.alloc(4);
    length.writeUInt32LE(body.length, 0);
    const reply = new Promise<Json>((resolve) => this.waiting.push(resolve));
    this.socket.write(Buffer.concat([length, body]));
    return reply;
  }

  /** One sealed action: the opened reply, or the refusal it carried. */
  async call(action: string, message: Json = {}): Promise<Json> {
    const nonce = nacl.randomBytes(24);
    const plain = Buffer.from(JSON.stringify({ ...message, action }), "utf8");
    const reply = await this.roundTrip({
      action,
      message: b64(nacl.box(plain, nonce, this.hostKey, this.keys.secretKey)),
      nonce: b64(nonce),
      clientID: CLIENT_ID,
    });
    if (reply.errorCode !== undefined) {
      throw new Refused(Number(reply.errorCode), String(reply.error ?? ""));
    }
    const expected = next(nonce);
    const opened = nacl.box.open(
      unb64(String(reply.message)),
      expected,
      this.hostKey,
      this.keys.secretKey,
    );
    if (!opened) throw new Error(`[e2e] a reply to ${action} did not open`);
    const inner = JSON.parse(Buffer.from(opened).toString("utf8")) as Json;
    if (inner.success !== "true" || inner.action !== action) {
      throw new Error(`[e2e] a bad reply to ${action}: ${JSON.stringify(inner)}`);
    }
    return inner;
  }

  /** Ask to be let in as an agent; settles once the app's dialog is answered. */
  associate(platform = "claude-code"): Promise<Json> {
    return this.call("associate", {
      key: b64(this.keys.publicKey),
      idKey: this.idKey,
      hello: { kind: "agent", platform },
    });
  }

  /** Ask to log into `origin` with any login of the lockbox, under `runId` if given. */
  requestLogin(origin: string, runId?: string, reason?: string): Promise<Json> {
    return this.call("request-grant", {
      ...(runId ? { runId } : {}),
      ...(reason ? { reason } : {}),
      items: "lockbox",
      targets: [{ origin }],
      modes: ["fill"],
    });
  }

  endGrant(grantId: string): Promise<Json> {
    return this.call("end-grant", { grantId });
  }

  close(): void {
    this.socket.destroy();
  }
}
