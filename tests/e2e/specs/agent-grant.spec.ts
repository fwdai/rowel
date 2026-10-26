import { createLogin, resetEmpty, unlock, waitFor } from "../helpers";
import { Agent, Refused } from "../helpers/agent";

/**
 * The grant dialog, raised the way a coding agent raises it: a client of
 * kind `agent` on the app's broker socket (helpers/agent.ts), let in through
 * the associate dialog, asks to log into a site the vault has a login for,
 * and the reply it gets is whatever the dialog was answered with.
 */

const MASTER_PASSWORD = "Gr4nt$dialog!e2e";
const SITE = "https://github.com";
const GRANT_DENIED = 40;

const testid = (id: string) => $(`[data-testid="${id}"]`);

// The extension host listening, the Agents switch and the lockbox's agent
// access, through the dev-only bridge (src/lib/e2e.ts).
async function turnAgentsOn(): Promise<void> {
  const failure = await browser.executeAsync(function (done: (error: string | null) => void) {
    (window as unknown as { __e2eAgents: () => Promise<void> })
      .__e2eAgents()
      .then(() => done(null), (error: unknown) => done(String(error)));
  });
  if (failure) throw new Error(`[e2e] turning agents on failed: ${failure}`);
}

async function dialogUp(): Promise<void> {
  await waitFor("agent-grant-dialog", 30_000);
}

async function dialogGone(): Promise<void> {
  await testid("agent-grant-dialog").waitForExist({ reverse: true, timeout: 15_000 });
}

describe("agent grant dialog", () => {
  let agent: Agent;

  before(async () => {
    await resetEmpty(MASTER_PASSWORD);
    await unlock(MASTER_PASSWORD);
    await createLogin({
      title: "GitHub",
      username: "octocat",
      password: "e2e-Secret-91!",
      website: SITE,
    });
    await turnAgentsOn();

    agent = await Agent.connect();
    const associated = agent.associate("claude-code");
    await waitFor("browser-associate-modal", 30_000);
    await testid("browser-associate-allow").click();
    await associated;
  });

  after(() => agent?.close());

  it("names the agent, its reason and the site, offers this run, and Once approves", async () => {
    const reply = agent.requestLogin(SITE, "run-e2e-1", "check the deploy");
    await dialogUp();

    await expect(testid("agent-grant-title")).toHaveText(
      "Claude Code wants to log into https://github.com",
    );
    await expect(testid("agent-grant-reason")).toHaveText("Its reason: “check the deploy”");
    await expect(testid("agent-grant-ends-session")).toBeDisplayed();
    for (const control of [
      "agent-grant-once",
      "agent-grant-run",
      "agent-grant-standing",
      "agent-grant-deny",
      "agent-grant-days",
    ]) {
      await expect(testid(control)).toBeDisplayed();
    }
    await expect(testid("agent-grant-run-note")).toHaveText(/at most 8 hours/);

    await testid("agent-grant-once").click();
    const granted = await reply;
    await dialogGone();
    if (granted.state !== "active") {
      throw new Error(`[e2e] Once left the grant ${JSON.stringify(granted)}`);
    }
    // Ended, so the next ask is a new dialog rather than served under it.
    await agent.endGrant(String(granted.grantId));
  });

  it("Deny refuses the request", async () => {
    const reply = agent.requestLogin(SITE, "run-e2e-2");
    await dialogUp();
    await testid("agent-grant-deny").click();

    const refusal = await reply.then(
      () => null,
      (error: unknown) => error,
    );
    await dialogGone();
    if (!(refusal instanceof Refused) || refusal.code !== GRANT_DENIED) {
      throw new Error(`[e2e] Deny was answered with ${String(refusal)}`);
    }
  });

  it("does not offer this run to a request that names no run", async () => {
    const reply = agent.requestLogin(SITE);
    await dialogUp();
    await expect(testid("agent-grant-once")).toBeDisplayed();
    await expect(testid("agent-grant-run")).not.toBeExisting();
    await expect(testid("agent-grant-run-note")).not.toBeExisting();

    await testid("agent-grant-deny").click();
    await reply.catch(() => undefined);
    await dialogGone();
  });
});
