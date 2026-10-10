import { call } from './client'

export interface GeneratorOptions {
  length: number
  numbers: boolean
  symbols: boolean
  uppercase: boolean
  lowercase?: boolean
  exclude?: string
  excludeSimilarCharacters?: boolean
  strict?: boolean
}

export const generatePassword = (options: GeneratorOptions): Promise<string> =>
  call('generate_password', { options })

// A generated ed25519 keypair, keyed like the `ssh` draft so it can be handed
// to `startEntry` as a prefill without a translation step — which is why it is
// an alias rather than an interface: only the former gets the implicit index
// signature that `Record<string, string>` wants.
export type SshKeyPair = {
  privateKey: string
  publicKey: string
  fingerprint: string
}

// A new unencrypted ed25519 keypair. `comment` is the label on the public line.
export const generateSshKey = (comment?: string): Promise<SshKeyPair> =>
  call('generate_ssh_key', { comment: comment || null })

export interface OtpResult {
  code: string
  time: number // seconds left in the current window
  period: number // how long that window is — 30s unless the seed said otherwise
}

// `secret` is a bare base32 seed or a whole otpauth:// URI; the backend reads
// the digits, period and algorithm off the latter.
export const generateOtp = (secret: string): Promise<OtpResult> =>
  call('generate_otp', { secret })

export interface AuditItem {
  score: number // zxcvbn strength, 0 (weakest) to 4 (strongest)
  isWeak: boolean
  isRepeating: boolean
  breached: boolean // exposed in a known breach (only when the breach check is enabled)
}

// Keyed by entry id; only entries that have a password are included.
export type Audit = Record<string, AuditItem>

export const getAudit = (checkBreaches: boolean): Promise<Audit> =>
  call('get_audit', { checkBreaches })

// What the OS text recognizer read out of one image: the kind of secret it is,
// and its fields keyed exactly like that kind's draft.
/**
 * The OS file dialog for a file the backend will then read, opened from Rust
 * rather than through the dialog plugin here: `scanImage` and `readEnvFile`
 * only read a path the backend itself watched the user choose, and the picker
 * that runs there is what makes the choice one it saw. `null` when the dialog
 * was dismissed.
 *
 * `label` names the filter in the dialog's own chrome, so it comes from the
 * catalogue the webview owns. `'env'` takes any file — an env file may be
 * named anything — and so does `'attachment'`.
 */
export const pickFileToRead = (
  kind: 'image' | 'env' | 'attachment',
  label?: string
): Promise<string | null> =>
  call('pick_file', { kind, label: label ?? null })

export interface ScanResult {
  kind: 'card' | 'identity'
  fields: Record<string, string>
}

/**
 * Recognize a card or an identity document in the image at `path`.
 *
 * The path is the one the user already has (a drop, or `pickFileToRead`) — the
 * image is read where it lies and never copied. A path from anywhere else is
 * refused: the backend reads only what it saw the user choose. Rejects
 * `unrecognized` when the text is there but says neither.
 */
export const scanImage = (path: string): Promise<ScanResult> => call('scan_image', { path })

/**
 * Recognize a card or an identity document in a photo the webview holds —
 * the one the phone's camera just took through the capture input, which has
 * no path on disk. The bytes go over as the request's raw body, not as JSON.
 * No grant is involved: nothing on disk is read for them.
 */
export const scanImageBytes = (image: Uint8Array): Promise<ScanResult> =>
  call('scan_image_bytes', image)

// The site's favicon as a data: URI, or null when it has none. The backend
// fetches straight from the entry's own host (never a third-party favicon
// service) and caches on disk, so repeat calls are a file read.
export const fetchFavicon = (host: string): Promise<string | null> =>
  call('fetch_favicon', { host })

export const copyToClipboard = (value: string, clearAfterMs?: number): Promise<void> =>
  call('copy_to_clipboard', { value, clearAfterMs })
