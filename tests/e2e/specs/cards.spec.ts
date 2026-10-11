import {
  createCard,
  entryItems,
  resetEmpty,
  unlock,
  waitFor,
} from "../helpers";

// The card detail is an interactive card face, not a list of rows: every value
// copies on click and one eye reveals number/CVC/PIN together. These specs
// cover that face plus the brand mark the number is derived into at save.

const MASTER_PASSWORD = "Hq6&tWn4jZv8mB!e";

const VISA = {
  title: "Visa Everyday",
  number: "4111111111111111",
  month: "04",
  year: "2029",
  cvc: "123",
  pin: "4321",
  name: "ADA LOVELACE",
};

const MASTERCARD = {
  title: "Mastercard Travel",
  number: "5555555555554444",
  month: "11",
  year: "2030",
  cvc: "456",
  pin: "8765",
};

const MASKED_NUMBER = "•••• •••• •••• 1111";
const REVEALED_NUMBER = "4111 1111 1111 1111";

const value = (field: string) => $(`[data-testid="entry-value-${field}"]`);
const brandMark = (slug: string) => $(`svg[aria-label="${slug}"]`);

describe("card entries", () => {
  before(async () => {
    await resetEmpty(MASTER_PASSWORD);
    await unlock(MASTER_PASSWORD);
    await createCard(VISA);
    // Saving selects the new entry, so the face is what the detail pane shows.
    await waitFor("entry-value-number");
  });

  it("renders the saved values on the card face, secrets masked", async () => {
    await expect(value("number")).toHaveText(MASKED_NUMBER);
    await expect(value("cvc")).toHaveText("•••");
    await expect(value("pin")).toHaveText("••••");
    // Expiry and cardholder are not secrets — they read plainly.
    await expect(value("expires")).toHaveText("04/29");
    await expect(value("name")).toHaveText(VISA.name);
  });

  it("shows the brand mark derived from the number", async () => {
    await brandMark("visa").waitForDisplayed({ timeout: 15_000 });
    await expect(brandMark("visa")).toBeDisplayed();
  });

  it("reveals number, CVC and PIN together and hides them again", async () => {
    await waitFor("card-reveal-button");
    await $('[data-testid="card-reveal-button"]').click();

    await expect(value("number")).toHaveText(REVEALED_NUMBER);
    await expect(value("cvc")).toHaveText(VISA.cvc);
    await expect(value("pin")).toHaveText(VISA.pin);

    await $('[data-testid="card-reveal-button"]').click();

    await expect(value("number")).toHaveText(MASKED_NUMBER);
    await expect(value("cvc")).toHaveText("•••");
    await expect(value("pin")).toHaveText("••••");
  });

  it("copies the number on click and confirms in place", async () => {
    const mark = $('[data-testid="copied-mark"]');
    // The check is mounted only while a copy is fresh, so at rest there is no
    // element at all — which `not.toBeDisplayed` covers either way.
    await expect(mark).not.toBeDisplayed();

    await waitFor("entry-value-number");
    await value("number").click();

    await expect(mark).toBeDisplayed();
    // The face confirms on the value itself; the app-level pill stays down.
    await expect($('[data-testid="copy-toast"]')).not.toBeDisplayed();

    // Let it time out rather than leaking a visible check into the next test.
    await mark.waitForDisplayed({ reverse: true, timeout: 10_000 });
  });

  it("derives the mastercard mark from a mastercard number", async () => {
    await createCard(MASTERCARD);
    await browser.waitUntil(async () => (await entryItems()).length === 2, {
      timeout: 15_000,
      timeoutMsg: "the second card never reached the list",
    });

    await brandMark("mastercard").waitForDisplayed({ timeout: 15_000 });
    await expect(brandMark("mastercard")).toBeDisplayed();
    await expect(value("number")).toHaveText("•••• •••• •••• 4444");
  });
});
