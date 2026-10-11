import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import type { EntryMeta } from '@/api/types'
import { useUi, useVault, setView } from '@/store'
import { withEntries, loginEntry, loginMeta } from './utils'
import { calls, mockCommand } from './ipc'

const note = (id: string, title: string): EntryMeta =>
  ({ id, type: 'note', title, tags: [], urlHost: '', favorite: false })
const card = (id: string, title: string): EntryMeta =>
  ({ id, type: 'card', title, tags: [], urlHost: '', favorite: false })

beforeEach(() => vi.clearAllMocks())

describe('Main', () => {
  const seed = () =>
    withEntries([loginMeta({ id: 'l1', title: 'Google' }), note('n1', 'Journal'), card('c1', 'Visa')])

  it('lists every kind together by default', () => {
    seed()
    render(<Main />)
    // "All Items" is the landing view now — one flat, mixed-kind list.
    expect(screen.getByTestId('list-title')).toHaveTextContent('All Items')
    expect(screen.getByText('Google')).toBeInTheDocument()
    expect(screen.getByText('Journal')).toBeInTheDocument()
    expect(screen.getByText('Visa')).toBeInTheDocument()
    expect(screen.getAllByTestId('entry-item')).toHaveLength(3)
  })

  it('selects an entry and shows its details', async () => {
    // Details are revealed (decrypted) on demand for the selected entry.
    mockCommand('reveal_entry', () => loginEntry({ id: 'l1', title: 'Google' }))
    seed()
    render(<Main />)
    await userEvent.click(screen.getByText('Google'))
    expect(screen.getByRole('heading', { name: 'Google' })).toBeInTheDocument()
    // email only appears in the details pane, not the list row
    expect(await screen.findByText('contact@example.com')).toBeInTheDocument()
  })

  it('filters entries by search query', async () => {
    withEntries([loginMeta({ id: 'a', title: 'Airbnb' }), loginMeta({ id: 'g', title: 'Google' })])
    render(<Main />)

    await userEvent.type(screen.getByPlaceholderText('Search'), 'air')
    expect(screen.getByText('Airbnb')).toBeInTheDocument()
    expect(screen.queryByText('Google')).not.toBeInTheDocument()
  })

  // ⌘⏎ copies from the list without opening the entry, so there is no control
  // on screen to flash a check on: the app-level pill is the only feedback.
  it('confirms a ⌘⏎ copy from the search field with the pill', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'l1', title: 'Google' }))
    withEntries([loginMeta({ id: 'l1', title: 'Google' })])
    render(<Main />)

    await userEvent.click(screen.getByTestId('search-input'))
    await userEvent.keyboard('{Meta>}{Enter}{/Meta}')

    await vi.waitFor(() => expect(calls('copy_to_clipboard')).toHaveLength(1))
    expect(await screen.findByTestId('copy-toast')).toHaveTextContent('Copied to Clipboard')
  })

  it('focuses the one search field on ⌘F', async () => {
    seed()
    render(<Main />)
    expect(screen.getAllByTestId('search-input')).toHaveLength(1)

    await userEvent.keyboard('{Meta>}f{/Meta}')
    expect(screen.getByTestId('search-input')).toHaveFocus()
  })

  // `key` is what the layout prints — on ru-RU, ⌘F arrives as `а`. The chord is
  // read off the physical key, so a Cyrillic layout gets the same shortcuts.
  it('reads chords off the physical key, whatever the layout prints', async () => {
    seed()
    render(<Main />)

    act(() => {
      window.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'а', code: 'KeyF', metaKey: true, bubbles: true })
      )
    })
    expect(screen.getByTestId('search-input')).toHaveFocus()

    act(() => {
      window.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'т', code: 'KeyN', metaKey: true, bubbles: true })
      )
    })
    expect(useUi.getState().addPicker).toBe(true)
  })

  it('lets the open generator dialog swallow the shell chords', async () => {
    seed()
    render(<Main />)

    await userEvent.keyboard('{Meta>}g{/Meta}')
    expect(screen.getByTestId('generator-dialog')).toBeInTheDocument()

    // The dialog owns the keyboard: neither chord may reach the shell behind it.
    await userEvent.keyboard('{Meta>}f{/Meta}')
    expect(screen.getByTestId('search-input')).not.toHaveFocus()

    await userEvent.keyboard('{Meta>}n{/Meta}')
    expect(useUi.getState().addPicker).toBe(false)
  })

  it('edits the selected entry on ⌘E, and needs a selection to do it', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'l1', title: 'Google' }))
    seed()
    render(<Main />)

    // Nothing selected: the chord has nothing to edit.
    await userEvent.keyboard('{Meta>}e{/Meta}')
    expect(useVault.getState().editing).toBe(false)

    await userEvent.click(screen.getByText('Google'))
    await userEvent.keyboard('{Meta>}e{/Meta}')
    expect(useVault.getState().editing).toBe(true)
  })

  // jsdom implements no `inert` semantics, so the attribute itself is the
  // assertion: it is what takes the column's rows, chips and arrows out of the
  // browser's keyboard and pointer reach while a draft is open.
  it('takes the list column out of the keyboard while a draft is open', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'l1', title: 'Google' }))
    seed()
    render(<Main />)

    const column = screen.getByTestId('list-column')
    expect(column).not.toHaveAttribute('inert')

    await userEvent.click(screen.getByText('Google'))
    await userEvent.keyboard('{Meta>}e{/Meta}')
    expect(useVault.getState().editing).toBe(true)

    expect(column).toHaveAttribute('inert')
    // `pointer-events-none` only ever stopped the mouse.
    expect(column.className).not.toContain('pointer-events-none')
  })

  // The title is the scope menu's trigger in All Items; a row of it is a place.
  const openScope = () => userEvent.click(screen.getByTestId('list-title'))
  const pickScope = (id: string) => userEvent.click(screen.getByTestId(`scope-option-${id}`))

  it("narrows the list to one kind through the title's scope menu", async () => {
    seed()
    render(<Main />)

    await openScope()
    await pickScope('note')
    expect(screen.getByText('Journal')).toBeInTheDocument()
    expect(screen.queryByText('Google')).not.toBeInTheDocument()
    expect(screen.queryByText('Visa')).not.toBeInTheDocument()
    // The title follows the kind, and the menu has closed behind the pick.
    expect(screen.getByTestId('list-title')).toHaveTextContent('Secure notes')
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()

    // Reopened, the menu marks the open kind; All Items puts everything back.
    await openScope()
    expect(screen.getByTestId('scope-option-note')).toHaveAttribute('aria-checked', 'true')
    await pickScope('all')
    expect(screen.getByTestId('list-title')).toHaveTextContent('All Items')
    expect(screen.getAllByTestId('entry-item')).toHaveLength(3)
  })

  it('lists every kind in the scope menu with its count, the empty ones dimmed', async () => {
    seed()
    render(<Main />)
    await openScope()

    const count = (id: string) => screen.getByTestId(`scope-option-${id}-count`).textContent
    expect(count('all')).toBe('3')
    expect(count('login')).toBe('1')
    expect(count('card')).toBe('1')
    expect(count('note')).toBe('1')
    // A kind with nothing in it is still a place, just a quiet one.
    expect(count('ssh')).toBe('0')
    expect(screen.getByTestId('scope-option-ssh').className).toContain('opacity-45')
    expect(screen.getByTestId('scope-option-login').className).not.toContain('opacity-45')
  })

  it('titles the other views as plain text, with no scope menu', () => {
    seed()
    render(<Main />)

    act(() => setView('favorites'))
    expect(screen.getByTestId('list-title')).toHaveTextContent('Favorites')
    expect(screen.getByTestId('list-title')).not.toHaveAttribute('aria-haspopup')
  })

  // The health tile is parked (Settings › Audit deep-links into it now), so the
  // view is entered through the store rather than a rail press.
  it('switches to the vault health view and back to the rail', async () => {
    seed()
    render(<Main />)

    act(() => setView('health'))
    expect(screen.getByTestId('list-title')).toHaveTextContent('Vault Health')

    await userEvent.click(screen.getByTestId('view-items'))
    expect(screen.getByTestId('list-title')).toHaveTextContent('All Items')
    expect(screen.getAllByTestId('entry-item')).toHaveLength(3)
  })

  it('keeps the selected entry when the kind filter still admits it', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'l1', title: 'Google' }))
    seed()
    render(<Main />)

    await userEvent.click(screen.getByText('Google'))
    expect(useVault.getState().currentId).toBe('l1')

    // Narrowing to the kind you are already reading must not close it.
    await openScope()
    await pickScope('login')
    expect(useVault.getState().currentId).toBe('l1')

    // Narrowing to a kind that would hide it does clear the selection.
    await openScope()
    await pickScope('card')
    expect(useVault.getState().currentId).toBeNull()
  })

  it('shows the empty-vault hero in the detail pane when there are no entries', () => {
    withEntries([])
    render(<Main />)
    expect(screen.getByText('Your vault is empty')).toBeInTheDocument()
    expect(screen.getByTestId('create-first-entry-button')).toBeInTheDocument()
  })
})
