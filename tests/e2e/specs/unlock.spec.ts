import { resetEmpty, waitFor, waitForAppReady } from "../helpers";

// The lock screen: the wrong-password path, the Enter accelerator, the
// failed-attempt backoff, and where a long passphrase sits in the field.
//
// The backoff constants live in `src-tauri/src/commands/auth.rs`: the first
// FREE_ATTEMPTS (3) failures are free and the 4th locks for 2^1 = 2 seconds.
// Four attempts is therefore the cheapest way to observe a real lockout, and
// the wait it costs is ~2s. The state lives in a sidecar next to the DB, which
// `resetEmpty()` wipes — so this spec never leaks a lockout into another one.

const MASTER_PASSWORD = "Hn6@wKd3sYc8!fRt";
const WRONG_PASSWORD = "Nope-Not-The-One-1!";
const ENTER = "\uE007"; // the WebDriver key code for Enter

const FREE_ATTEMPTS = 3;

// More than twice what the lock field shows at once.
const LONG_PASSPHRASE =
  "a bunch of text that would never fit in this field at all ok";

const input = () => $('[data-testid="unlock-password-input"]');

/** Type into the lock field, proving the field really holds what we typed. */
async function typePassword(value: string): Promise<void> {
  const field = input();
  await field.waitForEnabled({ timeout: 30_000 });
  await field.setValue(value);
  await browser.waitUntil(async () => (await field.getValue()) === value, {
    timeout: 10_000,
    timeoutMsg: `the lock field never settled on the typed value`,
  });
}

/**
 * Where the lock field's text sits, measured in the real webview: the unit
 * suite runs in jsdom, which lays nothing out.
 */
function measureField() {
  return browser.execute(() => {
    const field = document.querySelector<HTMLInputElement>(
      '[data-testid="unlock-password-input"]',
    )!;
    // The input fills this frame, and the reveal button sits inside it.
    const frame = field.parentElement!;
    const box = frame.getBoundingClientRect();
    const spills = Array.from(frame.querySelectorAll("*")).some((el) => {
      const rect = el.getBoundingClientRect();
      return rect.left < box.left - 1 || rect.right > box.right + 1;
    });
    const padding = parseFloat(getComputedStyle(field).paddingRight);
    const overflow = field.scrollWidth - field.clientWidth;
    return {
      spills,
      textRight: field.getBoundingClientRect().right - padding,
      revealLeft: frame.querySelector("button")!.getBoundingClientRect().left,
      // How much text the field holds beyond what it shows, and how much of
      // that still lies past its right edge.
      overflow,
      pastTheEnd: overflow - field.scrollLeft,
    };
  });
}

/** Submit with Enter — the lock screen has no submit button of its own. */
async function submit(): Promise<void> {
  await browser.keys(ENTER);
}

describe("unlocking a vault", () => {
  it("rejects a wrong password and accepts the right one on Enter", async function () {
    // Argon2id runs unoptimised in the debug binary, so every attempt (right or
    // wrong) costs a real derive.
    this.timeout(120_000);

    await resetEmpty(MASTER_PASSWORD);

    await typePassword(WRONG_PASSWORD);
    await submit();

    await waitFor("unlock-error");
    await expect($('[data-testid="unlock-error"]')).toHaveText(
      "Incorrect Master Password",
    );
    await expect($('[data-testid="main-view"]')).not.toBeDisplayed();

    await typePassword(MASTER_PASSWORD);
    await submit();

    await waitForAppReady();
    await expect($('[data-testid="unlock-password-input"]')).not.toBeDisplayed();
  });

  it("keeps a passphrase longer than the field inside it, masked and revealed", async function () {
    this.timeout(120_000);

    await resetEmpty(MASTER_PASSWORD);
    await typePassword(LONG_PASSPHRASE);

    const masked = await measureField();
    expect(masked.spills).toBe(false);
    expect(masked.textRight).toBeLessThanOrEqual(masked.revealLeft);
    // The text scrolls instead, and has followed the caret to its end.
    expect(masked.overflow).toBeGreaterThan(0);
    expect(masked.pastTheEnd).toBeLessThanOrEqual(1);

    await $('[aria-label="Reveal passphrase"]').click();
    await browser.waitUntil(
      async () => (await input().getAttribute("type")) === "text",
      { timeout: 10_000, timeoutMsg: "the passphrase was never revealed" },
    );

    const revealed = await measureField();
    expect(revealed.spills).toBe(false);
    expect(revealed.textRight).toBeLessThanOrEqual(revealed.revealLeft);
    expect(revealed.overflow).toBeGreaterThan(0);
  });

  it("locks the field out after repeated failures, then unlocks once it expires", async function () {
    this.timeout(180_000);

    await resetEmpty(MASTER_PASSWORD);

    // The free attempts only ever report a bad password.
    for (let attempt = 1; attempt <= FREE_ATTEMPTS; attempt += 1) {
      await typePassword(`${WRONG_PASSWORD}-${attempt}`);
      await submit();
      await waitFor("unlock-error");
    }

    // One more crosses the threshold and arms the backoff.
    await typePassword(`${WRONG_PASSWORD}-${FREE_ATTEMPTS + 1}`);
    await submit();

    await waitFor("unlock-lockout");
    await expect($('[data-testid="unlock-lockout"]')).toHaveText(
      /Too many failed attempts\. Try again in \d+s/,
    );
    await expect(input()).toBeDisabled();

    // The countdown re-enables the field on its own; the correct password then
    // works, which also clears the sidecar.
    await input().waitForEnabled({ timeout: 30_000 });
    await waitFor("unlock-status");

    await typePassword(MASTER_PASSWORD);
    await submit();

    await waitForAppReady();
  });
});
