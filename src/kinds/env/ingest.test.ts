import { describe, it, expect } from 'vitest'
import { fileNameOf, isEnvDrop, isEnvFileName, mergeNewKeys, proposeTitle } from './ingest'

describe('fileNameOf', () => {
  it('takes the last segment of either separator', () => {
    expect(fileNameOf('/Users/me/code/api/.env')).toBe('.env')
    expect(fileNameOf('C:\\code\\api\\.env.local')).toBe('.env.local')
    expect(fileNameOf('.env')).toBe('.env')
  })
})

describe('proposeTitle', () => {
  it('names the project and the environment', () => {
    expect(proposeTitle('/Users/me/code/api/.env.production')).toBe('api · production')
    expect(proposeTitle('C:\\code\\api\\.env.local')).toBe('api · local')
  })

  it('is just the project for a bare .env', () => {
    expect(proposeTitle('/Users/me/code/api/.env')).toBe('api')
  })

  it('reads a suffixed name as the environment', () => {
    expect(proposeTitle('/Users/me/code/api/foo.env')).toBe('api · foo')
  })

  it('has only the environment when there is no folder', () => {
    expect(proposeTitle('.env.staging')).toBe('staging')
    expect(proposeTitle('.env')).toBe('')
  })
})

describe('isEnvFileName / isEnvDrop', () => {
  it('matches the dotfile family and the .env suffix', () => {
    expect(isEnvFileName('.env')).toBe(true)
    expect(isEnvFileName('.env.production')).toBe(true)
    expect(isEnvFileName('local.env')).toBe(true)
    expect(isEnvFileName('.envrc')).toBe(false)
    expect(isEnvFileName('export.csv')).toBe(false)
  })

  it('falls back to the content when the name says nothing', () => {
    expect(isEnvDrop('notes.txt', 'A=1\nB=2\n')).toBe(true)
    expect(isEnvDrop('notes.txt', 'just a note\nanother line\n')).toBe(false)
    expect(isEnvDrop('export.json', '{"a":1}\n')).toBe(false)
    // The name carries it even when the body is one line.
    expect(isEnvDrop('.env', 'A=1')).toBe(true)
  })
})

describe('mergeNewKeys', () => {
  const MINE = ['# Database', 'DATABASE_URL=postgres://mine', 'DB_POOL=10'].join('\n')

  it('appends only the keys that are absent, in the incoming order', () => {
    const theirs = ['DB_POOL=99', 'STRIPE_KEY=sk_live', 'DATABASE_URL=postgres://theirs', 'PORT=3000'].join(
      '\n'
    )
    expect(mergeNewKeys(MINE + '\n', theirs)).toBe(MINE + '\nSTRIPE_KEY=sk_live\nPORT=3000\n')
  })

  it('keeps the existing body byte for byte, trailing newline or not', () => {
    const merged = mergeNewKeys(MINE, 'PORT=3000')
    expect(merged.startsWith(MINE)).toBe(true)
    expect(merged).toBe(MINE + '\nPORT=3000\n')
    expect(mergeNewKeys(MINE + '\n', 'DB_POOL=1')).toBe(MINE + '\n')
  })

  it('quotes what needs quoting and takes a repeated incoming key once', () => {
    expect(mergeNewKeys('', 'MSG=hello world # yes\nMSG=again\nPATH=/a b')).toBe(
      'MSG=hello world\nPATH=/a b\n'
    )
    expect(mergeNewKeys('', 'MSG="two\\nlines"')).toBe('MSG="two\\nlines"\n')
  })
})
