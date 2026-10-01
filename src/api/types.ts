/**
 * The wire types more than one domain speaks: what an entry is, what the list
 * shows of it, and how a session opens. Crypto and the master key live entirely
 * in Rust — `unlock` returns decrypted data for display and mutations send
 * plaintext back to be re-encrypted.
 */

export type EntryType = 'login' | 'note' | 'card' | 'identity' | 'ssh' | 'env' | 'apikey'

// One free-form field on an entry: a label the user wrote and its value.
export interface ExtraField {
  label: string
  value: string
}

interface BaseEntry {
  id: string
  type: EntryType
  title: string
  tags?: string[]
  // Free-form label/value pairs, in the user's order. Any kind may carry them;
  // absent when there are none, so an entry without any is unchanged.
  extra?: ExtraField[]
  createdAt?: string
  updatedAt?: string
}

// A WebAuthn credential held by a login entry. Only P-256 ECDSA is supported,
// so there is no algorithm field. credentialId and userHandle are base64url and
// are carried verbatim — never re-encoded.
//
// The private key is deliberately absent: it is the one part of a credential
// that can sign, nothing here renders or edits it, and the authenticator that
// uses it lives in Rust. So a reveal blanks it (`Entry::redacted`) and a save
// sends the passkey back without it — the core puts it back from the row being
// replaced, matched by credentialId. Dropping a passkey from this list still
// drops it; what the webview cannot do is carry the key around.
export interface Passkey {
  credentialId: string
  rpId: string
  rpName?: string
  userHandle: string
  userName: string
  userDisplayName: string
  counter: number
  createdAt?: string // RFC3339
}

export interface LoginEntry extends BaseEntry {
  type: 'login'
  website: string
  username: string
  password: string
  email: string
  note: string
  otp: string // base32 TOTP secret, or an otpauth:// URI when the parameters are not the defaults
  password_updated_at?: string
  // Absent on entries with no passkeys, so a pre-passkey vault is unchanged.
  passkeys?: Passkey[]
}

export interface NoteEntry extends BaseEntry {
  type: 'note'
  note: string
}

export interface CardEntry extends BaseEntry {
  type: 'card'
  number: string
  month: string
  year: string
  cvc: string
  pin: string
  name: string
  note: string
}

// An ID document. `number` (the document number) and `personal_number` are the
// secrets; `name` is the holder's full name, kept whole rather than split. The
// three dates are ISO `YYYY-MM-DD` — the display pattern is a preference, never
// what is stored.
export interface IdentityEntry extends BaseEntry {
  type: 'identity'
  /** One of `passport`, `id_card`, `driver_license`, `residence_permit`, `other`. */
  doc_type: string
  name: string
  number: string
  /** Issuing country, ISO 3166-1 alpha-3 by preference but free text. */
  country: string
  nationality: string
  birth_date: string
  sex: string
  issue_date: string
  expiry_date: string
  authority: string
  personal_number: string
  note: string
}

// An SSH keypair. ed25519 only for now, so there is no algorithm field: the
// public line names it. `fingerprint` is derived from the key at generation or
// paste time and stored alongside, so the detail view can show it without
// parsing a PEM block. `passphrase` is what the key was protected with
// elsewhere — the app never encrypts the private key itself.
export interface SshEntry extends BaseEntry {
  type: 'ssh'
  privateKey: string // OpenSSH PEM
  publicKey: string // `ssh-ed25519 AAAA… comment`
  fingerprint: string // `SHA256:…`
  passphrase: string
  note: string
}

// A `.env` file. The file text is the one secret and the canonical form; the
// variables table the detail view shows is parsed from it at render time and
// written back into it, so there is never a second copy to keep in step and
// comments, blank lines and quoting round-trip untouched. `fileName` is the
// name of the file that was dropped in (`.env.production`), empty when the
// body was typed or pasted — it is not secret and is only kept so the file
// can be handed back out under its own name.
export interface EnvEntry extends BaseEntry {
  type: 'env'
  body: string // the file, verbatim
  fileName: string
  note: string
}

// An API key. The token is the one secret; the rest says where it is sent and
// what it may do there. `environment` is `test` or `production` (or empty),
// `scopes` the granted scopes as typed — space- or comma-separated, split for
// display — and the expiry rides in the identity's ISO date slot.
export interface ApiKeyEntry extends BaseEntry {
  type: 'apikey'
  apiKey: string
  environment: string
  baseUrl: string
  scopes: string
  expiry_date: string
  note: string
}

export type Entry =
  | LoginEntry
  | NoteEntry
  | CardEntry
  | IdentityEntry
  | SshEntry
  | EnvEntry
  | ApiKeyEntry

// Non-secret entry metadata for the list. Secrets live in the encrypted store
// and arrive only via revealEntry, one entry at a time.
export interface EntryMeta {
  id: string
  type: EntryType
  title: string
  tags: string[]
  urlHost: string
  // Card network slug ("visa", …) derived from the number at save time;
  // absent for non-cards and unrecognized numbers.
  cardBrand?: string
  // Starred by the user. Metadata rather than an entry field, so it is toggled
  // through setFavorite and never travels with the secrets.
  favorite: boolean
  // Whether the entry holds at least one passkey, derived from the payload at
  // save time so the list can mark the row without revealing anything. The
  // passkeys themselves only ever arrive via revealEntry.
  hasPasskey?: boolean
  // What an env file was called (".env.production") and how many variables it
  // defines, derived at save time like cardBrand so the list can subtitle the
  // row without a reveal. Absent on other kinds, on env rows saved before the
  // columns existed, and — for the name alone — on a pasted file.
  fileName?: string
  varCount?: number
  // A login's username, derived at save time like cardBrand. Absent on other
  // kinds and on rows not yet stamped.
  username?: string
  createdAt?: string
  updatedAt?: string
  // Present only on the tombstones listDeleted returns.
  deletedAt?: string
}

// A file attached to an entry, as its list shows it. The file itself never
// reaches the webview: it is read, sealed, unsealed and written in Rust.
export interface Attachment {
  id: string
  entryId: string
  name: string
  // Guessed from the name; absent when it says nothing.
  mime?: string
  // Bytes, before sealing.
  size: number
  createdAt?: string
}

export interface UnlockResult {
  entries: EntryMeta[]
  syncConfigured: boolean
}

// One encrypted database, with its own master password. Optional: an install
// has exactly one until the user makes a second. `name` is null for the primary
// workspace until it is renamed — `workspaceLabel` (see `@/lib/workspace`) is
// what the UI shows for it.
export interface Workspace {
  id: string
  name: string | null
  /**
   * The vault id of the vault inside, once a sync has settled one. The UI
   * shows nothing of it; the backend keeps it to turn away a Drive restore of a
   * pack that is already a workspace here.
   */
  vaultId?: string
  /**
   * Whether the workspace is connected to Drive right now, read off its own
   * token file when the list was built. Distinct from `vaultId`, which
   * outlives a disconnect: a pack on Drive is not a connection to it. Absent
   * from a backend built before the flag existed, which reads as not synced.
   */
  synced?: boolean
  /**
   * How many live entries the vault held when it was last open on this
   * device — the one thing about its contents readable while it is locked.
   * Absent for a vault not opened here since the count was first kept.
   */
  itemCount?: number
  /**
   * The tile colour the user picked: a palette key (see `lib/workspaceColor`),
   * never a hex value. Kept on this device only. Absent when none was chosen,
   * and the tile derives a hue from the id instead.
   */
  color?: string
}

// Which biometry this device gates with, straight from the OS (Apple reads
// `LAContext.biometryType`; everywhere else a fingerprint is the only kind
// there has ever been). The platform alone cannot answer it — iPhones and Touch
// ID iPads are the same build.
export type BiometryType = 'face' | 'touch' | 'none'

// How the enrolled vault key is gated. `protected` is OS-enforced (the macOS
// data-protection keychain releases the key only to Touch ID); `prompt` is
// app-enforced (we run the biometric check, then read a plain credential-store
// item). Decided once at enrollment and recorded, so the copy can be honest.
export type BiometricMode = 'protected' | 'prompt'
