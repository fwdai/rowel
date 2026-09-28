import { describe, it, expect } from 'vitest'
import { cardBrandOf } from './cardBrand'

// Mirror of the Rust tests in src-tauri/crates/core/src/cards.rs — the two detectors must
// agree (backend stores the slug at save time; this serves revealed views).
describe('cardBrandOf', () => {
  it('detects the major networks', () => {
    expect(cardBrandOf('4111 1111 1111 1111')).toBe('visa')
    expect(cardBrandOf('5500-0000-0000-0004')).toBe('mastercard')
    expect(cardBrandOf('2221000000000009')).toBe('mastercard') // 2-series
    expect(cardBrandOf('340000000000009')).toBe('amex')
    expect(cardBrandOf('6011000000000004')).toBe('discover')
    expect(cardBrandOf('6759649826438453')).toBe('maestro')
    expect(cardBrandOf('3530111333300000')).toBe('jcb')
    expect(cardBrandOf('36700102000000')).toBe('diners')
    expect(cardBrandOf('6212345678901265')).toBe('unionpay')
  })

  it('lets specific ranges win over broad ones', () => {
    expect(cardBrandOf('6011 0000')).toBe('discover') // not maestro's 60x
    expect(cardBrandOf('6200 0000')).toBe('unionpay') // not maestro
  })

  it('returns undefined for unknown or short input', () => {
    expect(cardBrandOf('')).toBeUndefined()
    expect(cardBrandOf('411')).toBeUndefined()
    expect(cardBrandOf('9999 9999')).toBeUndefined()
    expect(cardBrandOf(undefined)).toBeUndefined()
  })
})
