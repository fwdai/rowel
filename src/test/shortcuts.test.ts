import { describe, it, expect } from 'vitest'
import {
  assignShortcut,
  hintOf,
  refusal,
  resolveShortcuts,
  sanitizeShortcuts,
  shortcutAt
} from '@/lib/shortcuts'

// The shortcut table and the user's changes over it (`lib/shortcuts`).
describe('shortcut keys', () => {
  it('reads the defaults where nothing moved, and the new key where it did', () => {
    const keys = resolveShortcuts({ lock: 'KeyJ' })

    expect(keys.commands).toBe('KeyK')
    expect(keys.lock).toBe('KeyJ')
    expect(shortcutAt(keys, 'KeyJ')).toBe('lock')
    expect(shortcutAt(keys, 'KeyL')).toBeUndefined()
    expect(hintOf('KeyJ')).toBe('⌘J')
  })

  it('stores only what differs from the defaults', () => {
    expect(assignShortcut({}, 'lock', 'KeyJ')).toEqual({ lock: 'KeyJ' })
    expect(assignShortcut({ lock: 'KeyJ', add: 'KeyO' }, 'lock', 'KeyL')).toEqual({ add: 'KeyO' })
  })
})

describe('which keys an action may take', () => {
  it('takes letters only', () => {
    expect(refusal('lock', 'KeyJ')).toBeUndefined()
    expect(refusal('lock', 'Digit1')).toBe('invalid')
    expect(refusal('lock', 'Enter')).toBe('invalid')
  })

  it('never takes the OS’s own chords', () => {
    for (const code of ['KeyQ', 'KeyW', 'KeyH', 'KeyM'])
      expect(refusal('copyUrl', code)).toBe('system')
  })

  it('leaves a text field’s chords to the field, except as an action’s own default', () => {
    for (const code of ['KeyA', 'KeyC', 'KeyV', 'KeyX', 'KeyZ', 'KeyY'])
      expect(refusal('lock', code)).toBe('editing')
    expect(refusal('copySecret', 'KeyC')).toBeUndefined()
    expect(refusal('copySecretOtp', 'KeyY')).toBeUndefined()
    expect(refusal('copyUrl', 'KeyC')).toBe('editing')
  })
})

describe('the stored preference', () => {
  it('keeps known actions with a key and drops the rest', () => {
    expect(sanitizeShortcuts({ lock: 'KeyJ', nope: 'KeyP', toString: 'KeyP', edit: 5 })).toEqual({
      lock: 'KeyJ'
    })
    expect(sanitizeShortcuts(undefined)).toEqual({})
    expect(sanitizeShortcuts('KeyJ')).toEqual({})
  })
})
