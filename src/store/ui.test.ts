import { describe, it, expect, beforeEach } from 'vitest'
import {
  useUi,
  useVault,
  selectCurrent,
  setEntries,
  setCurrentEntry,
  setFilterTag,
  setFilterType,
  editEntry,
  setView,
  showTag,
  openSettings,
  closeSettings,
  lockSettings,
  setSettingsSection,
  resetUi
} from './index'
import { filterEntries } from '@/services/entries'
import type { EntryMeta } from '@/api/types'

const meta = (id: string, tags: string[] = []): EntryMeta =>
  ({ id, type: 'login', title: id, tags, urlHost: '', favorite: false })

const items = [meta('work', ['work']), meta('home', ['home']), meta('both', ['work', 'home'])]

const current = () => selectCurrent(useVault.getState())

const visible = () => {
  const { filterType, filterTag, query } = useUi.getState()
  return filterEntries(useVault.getState().items, { type: filterType, tag: filterTag, query }).map(
    e => e.id
  )
}

beforeEach(() => setEntries(items))

describe('setFilterTag', () => {
  it('starts unset, so every row is in scope', () => {
    expect(useUi.getState().filterTag).toBeNull()
    expect(visible()).toHaveLength(3)
  })

  it('narrows the rows to the entries carrying the tag', () => {
    setFilterTag('work')
    expect(visible()).toEqual(['work', 'both'])
  })

  it('restores every row when cleared', () => {
    setFilterTag('work')
    setFilterTag(null)
    expect(visible()).toHaveLength(3)
  })

  it('keeps a selection the new tag still shows', () => {
    setCurrentEntry('both')
    setFilterTag('work')

    expect(current()?.id).toBe('both')
  })

  it('drops a selection the new tag would hide, along with the open editor', () => {
    setCurrentEntry('home')
    editEntry()
    setFilterTag('work')

    expect(current()).toBeNull()
    expect(useVault.getState().editing).toBe(false)
  })

  it('keeps the selection when the filter is cleared', () => {
    setCurrentEntry('home')
    setFilterTag(null)

    expect(current()?.id).toBe('home')
  })

  it('is independent of the kind filter — both narrow the same list', () => {
    setFilterTag('work')
    setFilterType('login')

    expect(useUi.getState()).toMatchObject({ filterType: 'login', filterTag: 'work' })
    expect(visible()).toEqual(['work', 'both'])
  })
})

describe('openSettings', () => {
  beforeEach(resetUi)

  it('opens on General unless told where to go', () => {
    openSettings()
    expect(useUi.getState()).toMatchObject({ settings: true, settingsSection: 'language' })
  })

  it('opens on the section a deep link names — the sync chip, an opened backup', () => {
    openSettings('sync')
    expect(useUi.getState().settingsSection).toBe('sync')
  })

  it('reopens where it was left within a session, and on General after a reset', () => {
    openSettings('sync')
    closeSettings()
    openSettings()
    expect(useUi.getState().settingsSection).toBe('sync')

    resetUi()
    openSettings()
    expect(useUi.getState().settingsSection).toBe('language')
  })

  // The lock is the store's to keep: an opened backup or the palette asks from
  // outside the shell, where no control is there to be shown disabled.
  it('puts off a section asked for while locked until the lock is released', () => {
    openSettings('workspaces')
    lockSettings(true)

    openSettings('import')
    expect(useUi.getState()).toMatchObject({ settings: true, settingsSection: 'workspaces' })

    lockSettings(false)
    expect(useUi.getState()).toMatchObject({ settingsSection: 'import', settingsLocked: false })

    // Nothing is left over to move it again at the next release.
    setSettingsSection('sync')
    lockSettings(true)
    lockSettings(false)
    expect(useUi.getState().settingsSection).toBe('sync')
  })

  it('stays put under the lock when no section is named', () => {
    openSettings('workspaces')
    lockSettings(true)
    openSettings()
    lockSettings(false)
    expect(useUi.getState().settingsSection).toBe('workspaces')
  })
})

describe('views', () => {
  it('enters the Tags view with its tag in one step', () => {
    showTag('work')
    expect(useUi.getState()).toMatchObject({ view: 'tags', filterTag: 'work' })
  })

  it('drops the tag and the selection on the way to another view', () => {
    showTag('work')
    setCurrentEntry('both')

    setView('favorites')

    expect(useUi.getState()).toMatchObject({ view: 'favorites', filterTag: null })
    expect(current()).toBeNull()
  })

  it('drops the kind on the way to another view', () => {
    setFilterType('login')
    setView('favorites')

    expect(useUi.getState()).toMatchObject({ view: 'favorites', filterType: null })
  })

  it('drops the kind on the way into a tag, so the tag shows every kind', () => {
    setEntries([...items, { ...meta('card', ['work']), type: 'card' }])
    setFilterType('login')

    showTag('work')

    expect(useUi.getState()).toMatchObject({ view: 'tags', filterTag: 'work', filterType: null })
    expect(visible()).toEqual(['work', 'both', 'card'])
  })
})
