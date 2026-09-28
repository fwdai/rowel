/**
 * A base64 public key cut down to what can be compared by eye: its two ends.
 * Enough to tell two extensions apart, and to match the one a consent dialog
 * let in against the row it became in Settings.
 */
export const keyFingerprint = (key: string): string =>
  key.length <= 12 ? key : `${key.slice(0, 8)}…${key.slice(-4)}`
