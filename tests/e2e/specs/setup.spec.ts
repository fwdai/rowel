import {
  entryItems,
  reload,
  resetPristine,
  skipBiometricIfOffered,
  waitFor,
  waitForAppReady,
} from "../helpers";

// First-run setup: the two gates that stand between a fresh install and an
// unlocked app (strength, confirmation) plus the back/forward navigation around
// them. Both fields live on one screen now, so Continue is the click that has
// to hold them — and the backup question after it is what actually writes.
// Every test starts from `resetPristine()`, so none of them depends on what a
// previous one left on disk.

const MASTER_PASSWORD = "Qr7&tJm4vBxN9!pZ";
// Long enough to clear MIN_LENGTH (12) so the *strength* gate is what rejects
// it, not the length hint — zxcvbn scores a pure repeat at 0.
const WEAK_PASSWORD = "aaaaaaaaaaaaaa";

const startSetup = async (): Promise<void> => {
  await $('[data-testid="start-setup-button"]').click();
  await waitFor("setup-password-input");
};

describe("first-run setup", () => {
  it("blocks a weak master password at the strength gate", async () => {
    await resetPristine();
    await startSetup();

    await $('[data-testid="setup-password-input"]').setValue(WEAK_PASSWORD);
    await $('[data-testid="setup-confirm-password-input"]').setValue(
      WEAK_PASSWORD,
    );

    // The meter scores off a deferred macrotask, so the label fills in a beat
    // after the keystrokes — wait for the verdict rather than the element.
    await waitFor("password-strength");
    await expect($('[data-testid="password-strength-label"]')).toHaveText(
      /Very weak|Weak/,
    );

    await $('[data-testid="setup-continue-button"]').click();

    await waitFor("form-error");
    await expect($('[data-testid="form-error"]')).toHaveText(
      "Choose a stronger master password",
    );
    // Still on step one: the backup question was never reached.
    await expect(
      $('[data-testid="setup-skip-drive-button"]'),
    ).not.toBeDisplayed();
  });

  it("rejects a mismatched confirmation and creates no vault", async () => {
    await resetPristine();
    await startSetup();

    await $('[data-testid="setup-password-input"]').setValue(MASTER_PASSWORD);
    await $('[data-testid="setup-confirm-password-input"]').setValue(
      `${MASTER_PASSWORD}-typo`,
    );
    await $('[data-testid="setup-continue-button"]').click();

    await waitFor("form-error");
    await expect($('[data-testid="form-error"]')).toHaveText(
      "Passwords do not match",
    );
    await expect($('[data-testid="main-view"]')).not.toBeDisplayed();

    // Nothing was written: a reload re-runs `app_status` against disk, and
    // the app offers first-run setup again rather than an unlock screen.
    await reload();
    await waitFor("start-setup-button");
    await expect($('[data-testid="unlock-password-input"]')).not.toBeDisplayed();
  });

  it("goes back to the welcome screen and completes setup on the second run", async () => {
    await resetPristine();
    await startSetup();
    await $('[data-testid="setup-password-input"]').setValue(WEAK_PASSWORD);

    await $('[data-testid="go-back-button"]').click();
    await waitFor("start-setup-button");
    await expect($('[data-testid="setup-password-input"]')).not.toBeDisplayed();

    // Re-entering the flow starts it clean — the abandoned draft is not carried
    // back in.
    await startSetup();
    await expect($('[data-testid="setup-password-input"]')).toHaveValue("");

    await $('[data-testid="setup-password-input"]').setValue(MASTER_PASSWORD);
    await $('[data-testid="setup-confirm-password-input"]').setValue(
      MASTER_PASSWORD,
    );
    await $('[data-testid="setup-continue-button"]').click();

    // Step two: where the backup lives. Declining it is what creates the vault.
    await waitFor("setup-skip-drive-button");
    await $('[data-testid="setup-skip-drive-button"]').click();

    // Setup lands straight in an unlocked, empty vault — no extra unlock step.
    // The biometric question in between only appears on hardware that has one.
    await skipBiometricIfOffered();
    await waitForAppReady();
    await expect($('[data-testid="lock-vault-button"]')).toBeDisplayed();
    expect(await entryItems()).toHaveLength(0);
  });

  it("offers both ways back into data that already exists", async () => {
    await resetPristine();
    await waitFor("start-setup-button");

    await expect($('[data-testid="start-drive-button"]')).toBeDisplayed();
    await expect($('[data-testid="start-dropbox-button"]')).toBeDisplayed();
    await expect($('[data-testid="start-restore-button"]')).toBeDisplayed();
  });
});
