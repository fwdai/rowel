import { readSettings, resetEmpty, setSettings, unlock, waitFor } from "../helpers";

// The Settings modal: the nav, and the preference rows that are provable
// locally. Drive sync, the native file dialogs and the updater endpoint are all
// out of reach for the driver (see COVERAGE.md) — everything asserted here is
// either the persisted settings, or the DOM.

const MASTER_PASSWORD = "Kp9$wTz4nBv7qXe!";

const title = () => $('[data-testid="settings-modal"]').$("h1");

async function openSettings(): Promise<void> {
  await waitFor("settings-button");
  await $('[data-testid="settings-button"]').click();
  await waitFor("settings-modal");
}

async function section(name: string): Promise<void> {
  await $(`[data-testid="settings-nav-${name}"]`).click();
}

describe("settings", () => {
  before(async () => {
    // Every selector below is an English label; `reset()` seeds the locale.
    await resetEmpty(MASTER_PASSWORD);
    await unlock(MASTER_PASSWORD);
    await openSettings();
  });

  after(async () => {
    // The theme case below persists; leave the app on the default for the next
    // spec in the run.
    await setSettings({ theme: "light" });
  });

  it("opens on General and marks the active nav item", async () => {
    await expect(title()).toHaveText("General");
    await expect($('[data-testid="settings-nav-language"]')).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("walks every section from the nav", async () => {
    const sections: [string, string][] = [
      ["security", "Security"],
      ["audit", "Vault audit"],
      ["workspaces", "Workspaces"],
      ["import", "Import & export"],
      ["language", "General"],
      ["sync", "Sync & backup"],
    ];

    for (const [name, heading] of sections) {
      await section(name);
      await expect(title()).toHaveText(heading);
      await expect($(`[data-testid="settings-nav-${name}"]`)).toHaveAttribute(
        "aria-current",
        "page",
      );
    }
  });

  it("shows the bare version in the pinned footer", async () => {
    await expect($('[data-testid="app-version"]')).toHaveText(/^\d+\.\d+\.\d+/);
  });

  it("offers each sync provider while not connected", async () => {
    await section("sync");
    await expect($('[data-testid="settings-drive-row"]')).toHaveText(
      /Not connected/,
    );
    await expect($('[data-testid="settings-drive-connect"]')).toBeDisplayed();
    await expect($('[data-testid="settings-dropbox-connect"]')).toBeDisplayed();
  });

  it("keeps the encrypted backup form behind its control", async () => {
    await section("sync");
    await expect($('input[name="export_password"]')).not.toBeDisplayed();
    await $('[data-testid="settings-backup-row"]').$("button").click();
    await expect($('input[name="export_password"]')).toBeDisplayed();
  });

  it("persists the auto-lock and clipboard delays", async () => {
    await section("security");

    await $('[data-testid="settings-autolock-900"]').click();
    await expect($('[data-testid="settings-autolock-900"]')).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect((await readSettings()).autolockSecs).toBe(900);

    await $('[data-testid="settings-clipboard-0"]').click();
    expect((await readSettings()).clipboardTimeoutMs).toBe(0);

    await $('[data-testid="settings-clipboard-30000"]').click();
    expect((await readSettings()).clipboardTimeoutMs).toBe(30000);
  });

  it("persists the generator defaults", async () => {
    await section("security");
    await $('[data-testid="settings-generator-symbols"]').click();

    expect((await readSettings()).generator.symbols).toBe(false);

    // Put it back — the generator spec asserts against the default charset.
    await $('[data-testid="settings-generator-symbols"]').click();
  });

  it("turns breach monitoring on and leaves the always-on monitors alone", async () => {
    await section("audit");

    await expect($('[data-testid="settings-breach-toggle"]')).toHaveAttribute(
      "aria-checked",
      "false",
    );
    await $('[data-testid="settings-breach-toggle"]').click();
    await expect($('[data-testid="settings-breach-toggle"]')).toHaveAttribute(
      "aria-checked",
      "true",
    );

    // Weak and reused cannot be switched off, so they carry no control.
    const switches = await $('[data-testid="settings-modal"]')
      .$$('[role="switch"]')
      .getElements();
    expect(switches).toHaveLength(1);

    // The audit spec runs with the breach check off (no network in the suite).
    await $('[data-testid="settings-breach-toggle"]').click();
  });

  it("offers five app tiles and the other sources behind a sixth", async () => {
    await section("import");

    for (const key of [
      "1password",
      "lastpass",
      "enpass",
      "bitwarden",
      "keepass",
      "other",
      "swftx",
    ])
      await expect($(`[data-testid="import-tile-${key}"]`)).toBeDisplayed();

    // The rest are items of the sixth tile's menu, mounted only while it is open.
    await $('[data-testid="import-tile-other"]').click();
    for (const key of ["chrome", "cxf", "csv"])
      await expect($(`[data-testid="import-tile-${key}"]`)).toBeDisplayed();
    await browser.keys("Escape");
    await expect($('[data-testid="import-tile-chrome"]')).not.toExist();

    await expect($('[data-testid="import-dropzone"]')).toBeDisplayed();
  });

  it("switches the theme and the date format", async () => {
    await section("language");

    await $('[data-testid="settings-theme-dark"]').click();
    await expect($("html")).toHaveAttribute("data-theme", "dark");
    await $('[data-testid="settings-theme-light"]').click();
    await expect($("html")).toHaveAttribute("data-theme", "light");

    await $('[data-testid="settings-date-format-trigger"]').click();
    await $('[data-testid="settings-date-format-YYYY-MM-DD"]').click();
    expect((await readSettings()).dateFormat).toBe("YYYY-MM-DD");
    await $('[data-testid="settings-date-format-trigger"]').click();
    await $('[data-testid="settings-date-format-MM/DD/YYYY"]').click();
    expect((await readSettings()).dateFormat).toBe("MM/DD/YYYY");
  });

  it("closes from the header X", async () => {
    await $('[data-testid="modal-close"]').click();
    await $('[data-testid="settings-modal"]').waitForDisplayed({
      reverse: true,
      timeout: 10_000,
    });
  });
});
