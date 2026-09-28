// Card network detection from the leading digits (IIN/BIN ranges).
// TypeScript mirror of `src-tauri/crates/core/src/cards.rs` — the backend derives and
// stores the slug at save time for list metadata; this mirror serves views
// that already hold the revealed number. Keep the two in sync.

export type CardBrand =
  | 'visa'
  | 'mastercard'
  | 'amex'
  | 'discover'
  | 'jcb'
  | 'diners'
  | 'unionpay'
  | 'maestro'

const MARKED = new Set(['mastercard', 'maestro', 'visa', 'amex', 'discover'])

// Whether `brand` has a drawn mark in <CardBrandMark> — callers use this to
// pick a fallback glyph, since the component renders null for the rest.
export const hasBrandMark = (brand?: string | null): boolean =>
  !!brand && MARKED.has(brand)

/** Digits only: what the vault stores and what the brand test reads. */
export const cardDigits = (value: string): string => value.replace(/\D/g, '').slice(0, 19)

/** The 4-4-4-4 grouping a card number is printed in. */
export const groupCardNumber = (value: string): string =>
  cardDigits(value).match(/.{1,4}/g)?.join(' ') ?? ''

export const cardBrandOf = (number?: string): CardBrand | undefined => {
  const digits = (number ?? '').replace(/\D/g, '')
  if (digits.length < 4) return undefined
  const p2 = Number(digits.slice(0, 2))
  const p3 = Number(digits.slice(0, 3))
  const p4 = Number(digits.slice(0, 4))

  if (digits.startsWith('4')) return 'visa'
  if ((p2 >= 51 && p2 <= 55) || (p4 >= 2221 && p4 <= 2720)) return 'mastercard'
  if (p2 === 34 || p2 === 37) return 'amex'
  if (p4 === 6011 || p2 === 65 || (p3 >= 644 && p3 <= 649)) return 'discover'
  if (p4 >= 3528 && p4 <= 3589) return 'jcb'
  if ((p3 >= 300 && p3 <= 305) || p2 === 36 || p2 === 38 || p2 === 39) return 'diners'
  if (p2 === 62) return 'unionpay'
  if (p2 === 50 || (p2 >= 56 && p2 <= 58) || p2 === 67 || p2 === 63) return 'maestro'
  return undefined
}
