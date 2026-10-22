import { describe, it, expect } from 'vitest'
import type { Entry, EntryMeta } from '@/api/types'
import { eyebrow, listSubtitle } from './meta'

// What the list and the header can say about an env file without, and with, a
// reveal — the two lines the stamped metadata and the payload each feed.

const meta = (overrides: Partial<EntryMeta> = {}): EntryMeta => ({
  id: '1',
  type: 'env',
  title: 'api · production',
  tags: [],
  urlHost: '',
  favorite: false,
  ...overrides
})

const entry = (fileName: string): Entry => ({
  id: '1',
  type: 'env',
  title: 'api · production',
  body: 'A=1\n',
  fileName,
  note: ''
})

describe('env listSubtitle', () => {
  it('joins the file name and the count when both are stamped', () => {
    expect(listSubtitle(meta({ fileName: '.env.production', varCount: 14 }))).toBe(
      '.env.production · 14 vars'
    )
  })

  // A pasted file never had a name; the count still tells files apart.
  it('shows the count alone for a file without a name', () => {
    expect(listSubtitle(meta({ varCount: 14 }))).toBe('14 vars')
    expect(listSubtitle(meta({ varCount: 1 }))).toBe('1 var')
    expect(listSubtitle(meta({ varCount: 0 }))).toBe('0 vars')
  })

  it('shows the name alone when only it is known', () => {
    expect(listSubtitle(meta({ fileName: '.env.production' }))).toBe('.env.production')
  })

  // A row saved before the columns existed has neither until the unlock
  // backfill reaches it; the tags are the only non-secret line left.
  it('falls back to the tags when nothing is stamped', () => {
    expect(listSubtitle(meta({ tags: ['api', 'production'] }))).toBe('api · production')
    expect(listSubtitle(meta())).toBe('')
  })
})

describe('env eyebrow', () => {
  it('names the file once the entry is revealed', () => {
    expect(eyebrow({ ...entry('.env.production') })).toEqual({
      text: '.env.production',
      testid: 'entry-value-fileName'
    })
    expect(eyebrow({ ...entry('  .env  ') })).toEqual({ text: '.env', testid: 'entry-value-fileName' })
  })

  it('adds nothing for a file without a name', () => {
    expect(eyebrow({ ...entry('') })).toBeNull()
    expect(eyebrow({ ...entry('   ') })).toBeNull()
  })
})
