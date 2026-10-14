import { describe, it, expect } from 'vitest'
import { DOC_TYPES, docTypeOf, specOf, TEMPLATES } from './templates'

describe('TEMPLATES', () => {
  // Every document is at least a name and a number, and every row it lists has
  // to have somewhere to render from.
  it('describes every document type it declares', () => {
    for (const type of DOC_TYPES) {
      const rows = TEMPLATES[type]
      expect(rows.map(row => row.key)).toContain('name')
      expect(rows.map(row => row.key)).toContain('number')
      for (const row of rows) expect(specOf(row.key)).toBeDefined()
      // A duplicated key would render the same draft field twice.
      expect(new Set(rows.map(row => row.key)).size).toBe(rows.length)
    }
  })

  // The read view bands the rows by watching the group change down the list, so
  // a group that comes back later would open a second band of the same name.
  it('keeps every group contiguous', () => {
    for (const type of DOC_TYPES) {
      const groups = TEMPLATES[type].map(row => specOf(row.key).group)
      const bands = groups.filter((group, index) => group !== groups[index - 1])
      expect(new Set(bands).size).toBe(bands.length)
    }
  })
})

describe('docTypeOf', () => {
  it('reads the draft’s type, or falls back to a passport', () => {
    expect(docTypeOf({ type: 'identity', title: '', doc_type: 'id_card' })).toBe('id_card')
    expect(docTypeOf({ type: 'identity', title: '' })).toBe('passport')
    expect(docTypeOf({ type: 'identity', title: '', doc_type: 'spaceship' })).toBe('passport')
  })
})
