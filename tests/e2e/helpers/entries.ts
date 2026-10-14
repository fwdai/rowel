import { waitFor } from "./app";

/**
 * Entry creation through the real UI.
 *
 * Add always asks which kind: the rail button opens the picker modal
 * (`Main/AddSecret`) and the kind is chosen there, so the list filter no longer
 * has any say in what gets created.
 */

type Kind = "login" | "card" | "note" | "identity";

/** Pick a kind, or All Items, from the title's scope menu. */
export async function openKind(kind: Kind | "all"): Promise<void> {
  await waitFor("list-title");
  await $('[data-testid="list-title"]').click();
  await waitFor(`scope-option-${kind}`);
  await $(`[data-testid="scope-option-${kind}"]`).click();
}

/** The `doc_type` values the identity form offers, in its own order. */
export type DocType =
  | "passport"
  | "id_card"
  | "driver_license"
  | "residence_permit"
  | "other";

export interface LoginFields {
  title: string;
  username: string;
  password: string;
  website?: string;
  tags?: string[];
}

export interface CardFields {
  title: string;
  number: string;
  month: string;
  year: string;
  cvc: string;
  pin: string;
  name?: string;
  tags?: string[];
}

export interface NoteFields {
  title: string;
  note: string;
  tags?: string[];
}

/**
 * An ID document. Only `name` and `number` are on every document type; the rest
 * are filled when the chosen type has a row for them, so a caller passes what
 * its document actually carries. Dates go in as the user's own pattern
 * (MM/DD/YYYY on a freshly reset app) — the form normalizes them to ISO.
 */
export interface IdentityFields {
  title: string;
  /** Which document the picker is answered with; a passport when left out. */
  docType?: DocType;
  name: string;
  number: string;
  country?: string;
  nationality?: string;
  birthDate?: string;
  issueDate?: string;
  expiryDate?: string;
  sex?: string;
  authority?: string;
  personalNumber?: string;
  note?: string;
  tags?: string[];
}

// `entry-sheet` is now the editing container inside the detail pane rather
// than a sliding sheet — the testid is kept so every wait below still reads as
// "the editor is up".
//
// An identity is asked a second question before its form opens: the picker
// turns to a page of documents, and the form is cut from the one chosen. The
// form itself has no type switch any more.
async function openForm(kind: Kind, docType: DocType = "passport"): Promise<void> {
  await waitFor("add-entry-button");
  await $('[data-testid="add-entry-button"]').click();
  await waitFor("add-secret-modal");
  await $(`[data-testid="add-kind-${kind}"]`).click();
  if (kind === "identity") {
    await waitFor(`add-choice-${docType}`);
    await $(`[data-testid="add-choice-${docType}"]`).click();
  }
  await waitFor("entry-sheet");
  await $('input[name="title"]').waitForDisplayed({ timeout: 5_000 });
}

// Fields are addressed by `name` — the editor renders one borderless input per
// row (and one textarea for note bodies), so there is no per-field testid to
// depend on. The title is the pane's own heading input, still `name="title"`.
async function fill(name: string, value: string, tag = "input"): Promise<void> {
  await $(`${tag}[name="${name}"]`).setValue(value);
}

// Tags live in a chip input, not a plain field: each tag is committed with
// Enter (the component also commits on blur, but Enter keeps the caret in the
// input for the next one). Enter inside the editor is inert — it only submits
// on ⌘/Ctrl+Enter — so this never saves early.
async function fillTags(tags: string[] = []): Promise<void> {
  if (tags.length === 0) return;
  const input = $('[data-testid="tags-input"]');
  await input.waitForDisplayed({ timeout: 5_000 });
  for (const tag of tags) {
    await input.setValue(tag);
    await browser.keys("Enter");
  }
}

// Save and wait for the editor to give the pane back to the read view, which is
// the store's "write landed" signal (a failed write stays in edit mode with
// `entry-save-error`).
async function save(): Promise<void> {
  await $('[data-testid="save-entry-button"]').click();
  await $('[data-testid="entry-sheet"]').waitForDisplayed({
    reverse: true,
    timeout: 15_000,
  });
}

export async function createLogin(fields: LoginFields): Promise<void> {
  await openForm("login");
  await fill("title", fields.title);
  if (fields.website !== undefined) await fill("website", fields.website);
  await fill("username", fields.username);
  await fill("password", fields.password);
  await fillTags(fields.tags);
  await save();
}

export async function createCard(fields: CardFields): Promise<void> {
  await openForm("card");
  await fill("title", fields.title);
  // The card face IS the form: the number groups itself into 4-digit blocks as
  // it is typed (so it reads back as "4111 1111 1111 1111"), and the two Month
  // and Year boxes are now one MM/YY box that is split again on save. Callers
  // still pass the pair — typing "0429" is what produces month 04, year 29.
  await fill("number", fields.number);
  await fill("expiry", `${fields.month}${fields.year.slice(-2)}`);
  await fill("cvc", fields.cvc);
  await fill("pin", fields.pin);
  if (fields.name !== undefined) await fill("name", fields.name);
  await fillTags(fields.tags);
  await save();
}

export async function createNote(fields: NoteFields): Promise<void> {
  await openForm("note");
  await fill("title", fields.title);
  await fill("note", fields.note, "textarea");
  await fillTags(fields.tags);
  await save();
}

export async function createIdentity(fields: IdentityFields): Promise<void> {
  // The document is settled in the picker, before the form: it decides which
  // rows exist.
  await openForm("identity", fields.docType);
  await fill("title", fields.title);
  await fill("name", fields.name);
  await fill("number", fields.number);
  const optional: [string, string | undefined][] = [
    ["country", fields.country],
    ["nationality", fields.nationality],
    ["birth_date", fields.birthDate],
    ["sex", fields.sex],
    ["issue_date", fields.issueDate],
    ["expiry_date", fields.expiryDate],
    ["authority", fields.authority],
    ["personal_number", fields.personalNumber],
  ];
  for (const [name, value] of optional) {
    if (value !== undefined) await fill(name, value);
  }
  if (fields.note !== undefined) await fill("note", fields.note, "textarea");
  await fillTags(fields.tags);
  await save();
}

/**
 * Every entry row currently rendered in the list column.
 *
 * `getElements()` is what resolves the chainable `$$` result into a real array:
 * since WebdriverIO v9 a `ChainablePromiseArray` is no longer a promise, so
 * `await $$(...)` hands back the chainable itself and `.length` on it is a
 * `Promise<number>` that never equals a plain count.
 */
export function entryItems(): Promise<WebdriverIO.ElementArray> {
  return $$('[data-testid="entry-item"]').getElements();
}

/** Titles of the rows currently rendered, in list order. */
export async function visibleTitles(): Promise<string[]> {
  const rows = await $$('[data-testid="entry-item-title"]').getElements();
  const titles: string[] = [];
  for (const row of rows) titles.push(await row.getText());
  return titles;
}

/**
 * Wait for the list to settle on exactly `expected`, in order.
 *
 * The list is redrawn from a re-read after every write, so the titles are
 * polled rather than read once — never a fixed pause.
 */
export async function expectTitles(expected: string[]): Promise<void> {
  await browser.waitUntil(
    async () => {
      const titles = await visibleTitles();
      return (
        titles.length === expected.length &&
        titles.every((title, i) => title === expected[i])
      );
    },
    {
      timeout: 15_000,
      timeoutMsg: `list never settled on [${expected.join(", ")}]`,
    },
  );
  expect(await visibleTitles()).toEqual(expected);
}

/**
 * Click the list row with this title. Rows carry no per-entry testid, so they
 * are matched on their title text.
 */
export async function openEntry(title: string): Promise<void> {
  await waitFor("entry-item");
  for (const row of await entryItems()) {
    if ((await row.$('[data-testid="entry-item-title"]').getText()) === title) {
      await row.click();
      return;
    }
  }
  throw new Error(`[e2e] no list row titled "${title}"`);
}

/**
 * Take the open entry into the editor. Edit is a row of the header's overflow
 * menu rather than a button of its own, so this is two clicks; it returns once
 * the editing container is up, but not once the decrypted values have landed —
 * callers that type wait on the field they need.
 *
 * The first wait is not decoration: the read cluster mounts a beat after the
 * row is clicked, so clicking blind races the menu into existence.
 */
export async function startEdit(): Promise<void> {
  await waitFor("more-actions-button");
  await $('[data-testid="more-actions-button"]').click();
  await waitFor("edit-entry-button");
  await $('[data-testid="edit-entry-button"]').click();
  await waitFor("entry-sheet");
}

/**
 * Star or unstar whatever the detail pane is showing.
 *
 * The wait is not decoration: the header's read cluster mounts a beat after the
 * row is clicked, so clicking blind races the toggle into existence.
 */
export async function toggleFavorite(): Promise<void> {
  await waitFor("favorite-toggle");
  await $('[data-testid="favorite-toggle"]').click();
}
