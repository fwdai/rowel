import { countryCode } from '@/utils/countries'
import type { Doc } from './DocCell'

/**
 * A passport's machine-readable zone, generated from what the document already
 * says.
 *
 * ICAO 9303's TD3: two lines of 44, the fields laid out in fixed columns with
 * check digits over them, so the band is derived rather than stored — nothing
 * to fill in, and it stays true as the document is edited.
 *
 * The one liberty taken is the name. ICAO wants the primary identifier — the
 * surname — first, then `<<`, then the given names; the vault stores `name`
 * whole and deliberately, and guessing where to cut it is wrong for most of the
 * world. So the whole name goes in the name field, transliterated. A `surname`
 * row in `templates.ts` would make it exact.
 */

const FILLER = '<'

// ICAO's character values: digits are themselves, A–Z run 10–35, and the filler
// — along with anything that reached here unfolded — counts as zero.
const charValue = (ch: string): number => {
  if (ch >= '0' && ch <= '9') return ch.charCodeAt(0) - 48
  if (ch >= 'A' && ch <= 'Z') return ch.charCodeAt(0) - 55
  return 0
}

const WEIGHTS = [7, 3, 1]

/** ICAO's check digit over one field: values against a repeating 7-3-1, mod 10. */
export const checkDigit = (field: string): string =>
  String([...field].reduce((sum, ch, i) => sum + charValue(ch) * WEIGHTS[i % 3], 0) % 10)

/*
 * ICAO 9303 fixes a substitution for each Latin letter that is not a base letter
 * with a mark on it. Decomposition cannot touch those — there is nothing to take
 * off — so without this table they fall through to the filler and the name
 * quietly loses a letter: `ŁUKASZ` would print as `<UKASZ`, and a Danish or
 * Norwegian name would lose every `Ø`.
 */
const SUBSTITUTIONS: Record<string, string> = {
  Æ: 'AE',
  Ð: 'D',
  Ø: 'OE',
  Þ: 'TH',
  Đ: 'D',
  Ħ: 'H',
  Ĳ: 'IJ',
  Ł: 'L',
  Ŋ: 'N',
  Œ: 'OE',
  Ə: 'E'
}

// The Latin subset the band is printed in. The letters above are substituted
// whole; every other accented letter decomposes and drops its marks (`Ä` to `A`,
// which is ICAO's default reading); and everything the subset has no room for —
// spaces, hyphens, apostrophes — becomes the filler. Which is what a real
// document does to a name too: the band is the machine's reading of it, not the
// printed one. Composing to NFC first so a decomposed paste still matches the
// table, and upper-casing before it so `ß` has already become `SS`.
const transliterate = (text: string): string =>
  [...text.normalize('NFC').toUpperCase()]
    .map(ch => SUBSTITUTIONS[ch] ?? ch)
    .join('')
    .normalize('NFD')
    .replace(/\p{M}/gu, '')
    .replace(/[^A-Z0-9]+/g, FILLER)

// A field cut or padded to the width its column has.
const fit = (text: string, width: number): string =>
  transliterate(text).slice(0, width).padEnd(width, FILLER)

const ISO_DATE = /^(\d{4})-(\d{2})-(\d{2})$/

// The band's six-digit date. A value that is not a stored date — half typed, or
// never filled in — prints as filler rather than as a wrong date.
const yymmdd = (iso: string): string => {
  const match = ISO_DATE.exec(iso.trim())
  return match ? `${match[1].slice(2)}${match[2]}${match[3]}` : FILLER.repeat(6)
}

// The band has one column for sex and three values for it.
const sexOf = (value: string): string => {
  const first = value.trim().toUpperCase()[0]
  return first === 'M' || first === 'F' ? first : FILLER
}

/*
 * A secret stays a secret in the band.
 *
 * The document number and the personal number are the two fields the vault
 * encrypts, and the face masks them until the eye is pressed — so their columns
 * here are overwritten too, and so is every check digit computed from them. A
 * check digit is a function of the field it covers: left standing it would hand
 * back a digit of what the mask is holding, and the composite digit at the end
 * of the line covers the document number along with everything else.
 */
const DOT = '•'

const blank = (line: string, from: number, to: number): string =>
  line.slice(0, from) + DOT.repeat(to - from) + line.slice(to)

/*
 * The two state columns take an ISO 3166-1 alpha-3 code and nothing else.
 *
 * Both fields are free text — the form asks for a code and takes whatever is
 * typed — so anything that is not one prints as filler. Cutting it to three
 * characters instead would turn "United Kingdom" into `UNI`, which is not a
 * country: a wrong code in a machine-readable zone is worse than an obviously
 * absent one, because only the absent one admits it does not know.
 */
const state = (value: string): string => countryCode(value) ?? FILLER.repeat(3)

// TD3 — the passport data page. Line 1 is the document code, the issuing state
// and the name; line 2 is every other field in fixed columns, each followed by
// its check digit, and a composite digit over the lot.
export const mrz = (doc: Doc): string[] => {
  const issuer = state(doc.value('country'))
  // The band always names a nationality; a document that only says where it was
  // issued lends that, which is true of every document that omits the field.
  const nationality = countryCode(doc.value('nationality')) ?? issuer
  const number = fit(doc.value('number'), 9)
  const dob = yymmdd(doc.value('birth_date'))
  const expiry = yymmdd(doc.value('expiry_date'))
  const optional = fit(doc.value('personal_number'), 14)

  const fields =
    number +
    checkDigit(number) +
    nationality +
    dob +
    checkDigit(dob) +
    sexOf(doc.value('sex')) +
    expiry +
    checkDigit(expiry) +
    optional +
    checkDigit(optional)

  // The composite covers the number, the dates and the optional data with their
  // own check digits — but not the nationality or the sex between them.
  const composite = checkDigit(fields.slice(0, 10) + fields.slice(13, 20) + fields.slice(21, 43))
  const line = fields + composite

  return [
    `P${FILLER}${issuer}${fit(doc.value('name'), 39)}`,
    doc.shown ? line : blank(blank(line, 28, 44), 0, 10)
  ]
}
