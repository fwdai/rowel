import { describe, it, expect } from 'vitest'
import type { IdentityKey } from '../templates'
import type { Doc } from './DocCell'
import { checkDigit, mrz } from './mrz'

// The specimen holder ICAO 9303 uses throughout its own examples, so the bands
// below can be compared against the printed standard character for character.
//
// ICAO's own specimen issues from `UTO`, a fictional state, which this prints as
// filler like any other non-code (see "the state columns"). A real code is
// substituted so the rest of the line can still be compared byte for byte — the
// check digits do not cover either state column, so nothing else moves.
const ANNA: Partial<Record<IdentityKey, string>> = {
  name: 'ERIKSSON ANNA MARIA',
  country: 'GBR',
  nationality: 'GBR',
  birth_date: '1974-08-12',
  sex: 'F',
  expiry_date: '2012-04-15'
}

const doc = (fields: Partial<Record<IdentityKey, string>>, shown = true): Doc => ({
  value: key => fields[key] ?? '',
  shown,
  toggle: () => {}
})

describe('checkDigit', () => {
  // The digits printed on ICAO's own specimens for these fields.
  it('weights the field 7-3-1 and takes the remainder', () => {
    expect(checkDigit('L898902C3')).toBe('6')
    expect(checkDigit('D23145890')).toBe('7')
    expect(checkDigit('740812')).toBe('2')
    expect(checkDigit('120415')).toBe('9')
  })

  it('counts the filler as nothing', () => {
    expect(checkDigit('<<<<<<')).toBe('0')
  })
})

describe('mrz', () => {
  it('prints a passport as TD3', () => {
    const lines = mrz(doc({ ...ANNA, number: 'L898902C3', personal_number: 'ZE184226B' }))

    expect(lines).toEqual([
      'P<GBRERIKSSON<ANNA<MARIA<<<<<<<<<<<<<<<<<<<<',
      'L898902C36GBR7408122F1204159ZE184226B<<<<<10'
    ])
  })

  // Every line is a fixed width; a value too long for its column is cut and a
  // short one padded, so the columns after it never shift.
  it('holds the band to its width', () => {
    const long = { ...ANNA, name: 'X'.repeat(80), number: '1234567890123' }
    expect(mrz(doc(long)).every(line => line.length === 44)).toBe(true)
  })

  it('prints filler rather than a wrong date for a date not yet typed', () => {
    const lines = mrz(doc({ ...ANNA, expiry_date: '2012-0', number: 'L898902C3' }))
    expect(lines[1].slice(21, 27)).toBe('<<<<<<')
  })

  // Upper-casing before the filter is what expands the eszett, which is the
  // substitution ICAO asks for — the same as its own ß → SS.
  it('folds an accented name onto the letters the band has', () => {
    const [line] = mrz(doc({ ...ANNA, name: 'Zoë Müller-Voß', number: 'L898902C3' }))
    expect(line).toBe('P<GBRZOE<MULLER<VOSS<<<<<<<<<<<<<<<<<<<<<<<<')
  })

  /*
   * A letter that is not a base letter with a mark on it has nothing to
   * decompose, so it needs ICAO's substitution or it falls through to the filler
   * and the name silently loses it.
   */
  it('substitutes the letters that do not decompose rather than dropping them', () => {
    // The name's own column: the 39 after the document code and the state.
    const name = (value: string) =>
      mrz(doc({ ...ANNA, name: value, number: 'L898902C3' }))[0].slice(5).replace(/<+$/, '')

    expect(name('Łukasz')).toBe('LUKASZ')
    expect(name('Ørsted')).toBe('OERSTED')
    expect(name('Æblerød')).toBe('AEBLEROED')
    expect(name('Þórsdóttir')).toBe('THORSDOTTIR')
    // A decomposed paste is the same letter and has to read as one.
    expect(name('Ł'.normalize('NFD'))).toBe(name('Ł'))
  })

  it('lends the issuing country as the nationality when there is none', () => {
    const lines = mrz(doc({ ...ANNA, nationality: '', number: 'L898902C3' }))
    expect(lines[1].slice(10, 13)).toBe('GBR')
  })

  /*
   * The country fields are free text. A value that is not an ISO 3166-1 alpha-3
   * code has to print as filler rather than be cut down to three characters: a
   * band that claims `UNI` is wrong in a way a band that says `<<<` is not.
   */
  describe('the state columns', () => {
    it('prints filler for a country that is not a code', () => {
      const lines = mrz(doc({ ...ANNA, country: 'United Kingdom', number: 'L898902C3' }))
      expect(lines[0].slice(2, 5)).toBe('<<<')
    })

    it('takes a real code however it was typed', () => {
      const lines = mrz(doc({ ...ANNA, country: ' gbr ', number: 'L898902C3' }))
      expect(lines[0].slice(2, 5)).toBe('GBR')
    })

    it('falls back to the issuer rather than to a mistyped nationality', () => {
      const lines = mrz(doc({ ...ANNA, country: 'GBR', nationality: 'British', number: 'L898902C3' }))
      expect(lines[1].slice(10, 13)).toBe('GBR')
    })
  })

  /*
   * The document number and the personal number are the vault's two encrypted
   * identity fields. A band that printed them while the face masks them would
   * hand both straight back — and so would a check digit computed over them,
   * which is why the digits go under the mask with the fields they cover.
   */
  describe('while the number is masked', () => {
    const secrets = { number: 'L898902C3', personal_number: 'ZE184226B' }

    it('prints neither secret nor any digit derived from one', () => {
      const [, line] = mrz(doc({ ...ANNA, ...secrets }, false))
      const [, open] = mrz(doc({ ...ANNA, ...secrets }))

      expect(line).toBe('••••••••••GBR7408122F1204159••••••••••••••••')
      // Whatever is still legible is legible on the revealed band too: the mask
      // only ever covers, it never rewrites what it leaves showing.
      expect(line).toHaveLength(open.length)
      expect(line.slice(10, 28)).toBe(open.slice(10, 28))
    })

    it('keeps no trace of the number in the masked band', () => {
      const masked = mrz(doc({ ...ANNA, ...secrets }, false)).join('')
      expect(masked).not.toContain('L898902C3')
      expect(masked).not.toContain('ZE184226B')
    })
  })
})
