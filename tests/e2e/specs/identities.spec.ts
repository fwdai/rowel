import {
  createIdentity,
  createLogin,
  expectTitles,
  resetEmpty,
  startEdit,
  unlock,
  waitFor,
  openKind,
} from "../helpers";

// ID documents. What makes this kind different from the other three is that the
// form is cut from the document type: a passport asks for a nationality and a
// personal number, a driving licence asks for neither. The document is chosen
// in the add picker before the form opens, so these specs cover that second
// page as much as the round trip — plus the reveal on the document number,
// which is the one field here the vault encrypts.

const MASTER_PASSWORD = "Zt8@qLm3vKp9!wRs";

const PASSPORT = {
  title: "UK Passport",
  name: "ADA LOVELACE",
  number: "X1234567",
  country: "GBR",
  nationality: "GBR",
  birthDate: "12/10/1815",
  expiryDate: "06/01/2035",
  personalNumber: "99-1815",
};

const LICENCE = {
  title: "Driving Licence",
  docType: "driver_license" as const,
  name: "ADA LOVELACE",
  number: "LOVEL815129AB9CD",
  country: "GBR",
  expiryDate: "03/01/2031",
};

const EDITED_LICENCE_NUMBER = "LOVEL815129AB9XY";

// A pair no document template has a row for.
const EXTRA = { label: "Blood type", value: "O+" };

const value = (field: string) => $(`[data-testid="entry-value-${field}"]`);
const revealNumber = () => $('[data-testid="reveal-number"]');

/**
 * The document number as text, with the mask lifted first.
 *
 * A masked read value is a fixed dot string standing in for the secret, not the
 * secret under a paint treatment — so nothing can be asserted about the number
 * until the eye has been pressed.
 */
async function readNumber(): Promise<string> {
  await waitFor("reveal-number");
  if ((await revealNumber().getAttribute("aria-label")) === "Reveal") {
    await revealNumber().click();
  }
  return value("number").getText();
}

/**
 * The document type, which reading shows in the eyebrow beside the kind.
 *
 * That is a mono micro-label, so it is on screen in the case the CSS gives it
 * rather than the one the catalog spells — the same reason the custom-field
 * label below is compared case-insensitively.
 */
async function expectDocType(label: string): Promise<void> {
  await waitFor("entry-value-doc_type");
  const shown = await value("doc_type").getText();
  expect(shown.toLowerCase()).toBe(label.toLowerCase());
}

/**
 * Re-open an entry so its secrets are decrypted afresh.
 *
 * Same reason as in notes.spec.ts: `useRevealed` keys the decrypt on the entry
 * id alone, so after an in-place save the detail pane keeps serving the pre-edit
 * plaintext. Opening a kind the selection is not clears it, which unmounts
 * the pane; coming back forces the reveal.
 */
async function reopen(title: string): Promise<void> {
  await openKind("login");
  await openKind("identity");
  await waitFor("entry-item");
  for (const row of await $$('[data-testid="entry-item"]').getElements()) {
    if ((await row.$('[data-testid="entry-item-title"]').getText()) === title) {
      await row.click();
      await waitFor("entry-value-number");
      return;
    }
  }
  throw new Error(`[e2e] no identity row titled "${title}"`);
}

describe("identity entries", () => {
  before(async function () {
    // Creation goes through the real UI on top of an Argon2id unlock.
    this.timeout(180_000);

    await resetEmpty(MASTER_PASSWORD);
    await unlock(MASTER_PASSWORD);
    await createIdentity(PASSPORT);
    // Saving selects the new entry, so the detail pane is already showing it.
    await waitFor("entry-value-number");
  });

  it("renders the saved passport, dates in the user's own pattern", async () => {
    await expectDocType("Passport");
    await expect(value("name")).toHaveText(PASSPORT.name);
    await expect(value("nationality")).toHaveText(PASSPORT.nationality);
    await expect(value("country")).toHaveText(PASSPORT.country);
    // Stored as ISO, read back as MM/DD/YYYY — the default pattern a reset app
    // boots with.
    await expect(value("birth_date")).toHaveText(PASSPORT.birthDate);
    await expect(value("expiry_date")).toHaveText(PASSPORT.expiryDate);
  });

  it("keeps the document number behind a reveal toggle", async () => {
    await waitFor("reveal-number");
    await expect(revealNumber()).toHaveAttribute("aria-label", "Reveal");

    await revealNumber().click();
    await expect(revealNumber()).toHaveAttribute("aria-label", "Hide");
    await expect(value("number")).toHaveText(PASSPORT.number);

    await revealNumber().click();
    await expect(revealNumber()).toHaveAttribute("aria-label", "Reveal");
  });

  it("cuts the form to the chosen document type", async () => {
    await createIdentity(LICENCE);
    await waitFor("entry-value-number");

    await expectDocType("Driver license");
    await expect(value("expiry_date")).toHaveText(LICENCE.expiryDate);
    expect(await readNumber()).toBe(LICENCE.number);
    // A licence has no nationality row, so the licence has no nationality —
    // the passport's is not something it inherited.
    await expect(value("nationality")).not.toBeExisting();
    await expect(value("personal_number")).not.toBeExisting();
  });

  it("round-trips an edit of the document number", async () => {
    await startEdit();

    // The editor opens on metadata and swaps in the decrypted values a tick
    // later; typing before that lands would be overwritten.
    const number = $('input[name="number"]');
    await expect(number).toHaveValue(LICENCE.number);
    await number.setValue(EDITED_LICENCE_NUMBER);

    await $('[data-testid="save-entry-button"]').click();
    await $('[data-testid="entry-sheet"]').waitForDisplayed({
      reverse: true,
      timeout: 15_000,
    });

    await reopen(LICENCE.title);
    expect(await readNumber()).toBe(EDITED_LICENCE_NUMBER);
  });

  it("filters the list down to identities", async () => {
    await createLogin({
      title: "Basalt Bank",
      username: "ada@example.com",
      password: "Seed-Basalt-Passphrase-2!",
    });

    await openKind("identity");

    // Newest write first, and the licence is the one that has been edited.
    await expectTitles([LICENCE.title, PASSPORT.title]);
    await expect($('[data-testid="list-title"]')).toHaveText("Identities");
    // Reopened, the scope menu marks the open kind and counts it.
    await $('[data-testid="list-title"]').click();
    await waitFor("scope-option-identity");
    await expect($('[data-testid="scope-option-identity"]')).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await expect($('[data-testid="scope-option-identity-count"]')).toHaveText("2");
    await browser.keys("Escape");
  });

  // What the template has no row for goes in a custom field, so the pair has to
  // survive a save and come back on the read view.
  it("round-trips a custom field the template has no row for", async () => {
    await reopen(PASSPORT.title);
    await startEdit();

    await $('[data-testid="add-extra-field"]').click();
    await $('input[name="extra-label-0"]').setValue(EXTRA.label);
    await $('input[name="extra-value-0"]').setValue(EXTRA.value);

    await $('[data-testid="save-entry-button"]').click();
    await $('[data-testid="entry-sheet"]').waitForDisplayed({
      reverse: true,
      timeout: 15_000,
    });

    await reopen(PASSPORT.title);
    // The label column is a mono micro-label, so it is on screen in the case the
    // CSS gives it rather than the one that was typed.
    const label = await $('[data-testid="entry-extra-label-0"]').getText();
    expect(label.toLowerCase()).toBe(EXTRA.label.toLowerCase());
    await expect($('[data-testid="entry-extra-value-0"]')).toHaveText(EXTRA.value);
  });

  // The document was settled before the form opened, so the editor only names
  // it: a caption over the rows, and no switch that could reshape them.
  it("names the document in the editor and offers no switch", async () => {
    await reopen(PASSPORT.title);
    await startEdit();
    await expect($('input[name="nationality"]')).toHaveValue(
      PASSPORT.nationality,
    );

    await waitFor("identity-doc-type");
    const caption = await $('[data-testid="identity-doc-type"]').getText();
    expect(caption.toLowerCase()).toBe("passport");
    await expect($('[data-testid="identity-doc-type-driver_license"]')).not.toBeExisting();

    await $('[data-testid="cancel-entry-button"]').click();
    await $('[data-testid="entry-sheet"]').waitForDisplayed({
      reverse: true,
      timeout: 15_000,
    });
  });
});
