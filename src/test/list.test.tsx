import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import ListColumn from '@/components/Main/Body/ListColumn'
import SortMenu from '@/components/Main/Body/List/SortMenu'
import { type Audit } from '@/api/tools'
import { useVault, useUi, setView, setFilterType, clearSession } from '@/store'
import { resetFavicons } from '@/hooks/useFavicon'
import { withEntries, loginEntry, loginMeta } from './utils'
import { calls, mockCommand, clearCalls } from './ipc'

// A fixed clock (Date only, so userEvent's real timers keep working) makes the
// recency buckets and the relative times deterministic.
const NOW = new Date(2024, 2, 14, 12, 0, 0)
const at = (...args: [number, number, number, number]) => new Date(...args).toISOString()

const entries = [
  loginMeta({ id: 'fresh', title: 'Zebra', updatedAt: at(2024, 2, 14, 9) }),
  loginMeta({ id: 'yday', title: 'Airbnb', updatedAt: at(2024, 2, 13, 18) }),
  loginMeta({ id: 'week', title: 'Monzo', updatedAt: at(2024, 2, 10, 12) }),
  loginMeta({ id: 'old', title: 'Basecamp', updatedAt: at(2024, 0, 12, 12) })
]

const titles = () =>
  screen
    .getAllByTestId('entry-item')
    .map(row => row.querySelector('.text-base')?.textContent)

const seed = (audit?: Audit) => withEntries(entries, audit)

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(NOW)
  resetFavicons()
  clearCalls('fetch_favicon')
})

afterEach(() => vi.useRealTimers())

describe('Entry list', () => {
  it('lists newest first as a flat list, no date headers', () => {
    seed()
    render(<ListColumn />)

    expect(titles()).toEqual(['Zebra', 'Airbnb', 'Monzo', 'Basecamp'])
    expect(screen.queryByText('Today')).not.toBeInTheDocument()
    expect(screen.queryByText('Earlier')).not.toBeInTheDocument()
  })

  it('shows a relative time on every row', () => {
    seed()
    render(<ListColumn />)
    expect(screen.getByText('3h')).toBeInTheDocument()
    expect(screen.getByText('4d')).toBeInTheDocument()
    expect(screen.getByText('01/12/2024')).toBeInTheDocument()
  })

  it('flags weak and reused entries from the audit already in the store', () => {
    const audit: Audit = {
      fresh: { score: 0, isWeak: true, isRepeating: false, breached: false },
      yday: { score: 2, isWeak: false, isRepeating: true, breached: false }
    }
    seed(audit)
    render(<ListColumn />)

    expect(screen.getByText('weak')).toBeInTheDocument()
    expect(screen.getByText('reused')).toBeInTheDocument()
    // Only the two audited entries carry a chip.
    expect(screen.getAllByTestId('entry-item')).toHaveLength(4)
  })

  it('swaps the glyph for the site favicon once it resolves, one fetch per host', async () => {
    const uri = 'data:image/png;base64,AAAA'
    mockCommand('fetch_favicon', () => uri)
    seed()
    render(<ListColumn />)

    await waitFor(() =>
      expect(document.querySelectorAll(`img[src="${uri}"]`)).toHaveLength(4)
    )
    // All four rows share one host — the lookup is deduped across them.
    expect(calls('fetch_favicon')).toHaveLength(1)
  })

  // The cache is keyed by hostname and holds the icons of whatever was on
  // screen, so it is session data like the rows themselves: leaving it would
  // carry one vault's sites into the next unlock, or into another workspace's.
  it('forgets the icons it cached when the vault locks', async () => {
    mockCommand('fetch_favicon', () => 'data:image/png;base64,AAAA')
    seed()
    const { unmount } = render(<ListColumn />)
    await waitFor(() => expect(calls('fetch_favicon')).toHaveLength(1))
    unmount()

    clearSession()
    seed()
    render(<ListColumn />)

    // A cache that survived would answer this host from memory.
    await waitFor(() => expect(calls('fetch_favicon')).toHaveLength(2))
  })

  it('keeps the type glyph when the host has no favicon', async () => {
    mockCommand('fetch_favicon', () => null)
    seed()
    render(<ListColumn />)

    await waitFor(() => expect(calls('fetch_favicon').length).toBeGreaterThan(0))
    expect(document.querySelector('img')).not.toBeInTheDocument()
  })

  it('shows the network mark on card rows that carry a brand', () => {
    withEntries([
      loginMeta({ id: 'c1', type: 'card', title: 'Company Visa', cardBrand: 'visa', urlHost: '' }),
      loginMeta({ id: 'c2', type: 'card', title: 'Mystery Card', urlHost: '' })
    ])
    setFilterType('card')
    render(<ListColumn />)

    expect(document.querySelector('svg[aria-label="visa"]')).toBeInTheDocument()
    // The brandless card falls back to the generic glyph, not an empty tile.
    expect(document.querySelectorAll('svg[aria-label]')).toHaveLength(1)
    setFilterType(null)
  })

  it('draws a passport with a known country as its cover', () => {
    const identity = { type: 'identity' as const, urlHost: '' }
    withEntries([
      loginMeta({ id: 'p1', title: 'My passport', docType: 'passport', docCountry: 'UKR', ...identity }),
      loginMeta({ id: 'p2', title: 'Licence', docType: 'driver_license', docCountry: 'UKR', ...identity }),
      loginMeta({ id: 'p3', title: 'Odd code', docType: 'passport', docCountry: 'XYZ', ...identity })
    ])
    setFilterType('identity')
    render(<ListColumn />)

    // Only the passport gets a cover: a licence has none, and a code that is no
    // country has no colour to draw.
    const covers = screen.getAllByTestId('passport-cover')
    expect(covers.map(cover => cover.dataset.country)).toEqual(['UKR'])
    expect(covers[0]).toHaveStyle({ backgroundColor: '#17305D' })
    setFilterType(null)
  })

  it('sorts alphabetically', async () => {
    // The sort control is one of the title row's actions, handed down by the
    // shell — the wide one sends exactly this.
    seed()
    render(<ListColumn actions={<SortMenu />} />)

    await userEvent.click(screen.getByTestId('sort-menu'))
    await userEvent.click(screen.getByText('Name (A–Z)'))

    expect(titles()).toEqual(['Airbnb', 'Basecamp', 'Monzo', 'Zebra'])
  })

  it('sorts by date created, which a later edit does not move', async () => {
    withEntries([
      loginMeta({ id: 'old', title: 'Basecamp', createdAt: at(2024, 0, 1, 9), updatedAt: at(2024, 2, 14, 11) }),
      loginMeta({ id: 'mid', title: 'Monzo', createdAt: at(2024, 1, 1, 9), updatedAt: at(2024, 1, 1, 9) }),
      loginMeta({ id: 'new', title: 'Zebra', createdAt: at(2024, 2, 1, 9), updatedAt: at(2024, 2, 1, 9) })
    ])
    render(<ListColumn actions={<SortMenu />} />)

    // Under Recently edited the oldest row leads: it was edited this morning.
    await userEvent.click(screen.getByTestId('sort-menu'))
    await userEvent.click(screen.getByText('Recently edited'))
    expect(titles()).toEqual(['Basecamp', 'Zebra', 'Monzo'])

    // Under Date created that edit counts for nothing: newest-made first.
    await userEvent.click(screen.getByTestId('sort-menu'))
    await userEvent.click(screen.getByText('Date created'))
    expect(titles()).toEqual(['Zebra', 'Monzo', 'Basecamp'])
  })

  // Read off the `hasPasskey` column, so a row is marked without the list
  // decrypting anything — and unmarked rows stay unmarked.
  it('marks only the logins that hold a passkey', () => {
    withEntries([
      loginMeta({ id: 'key', title: 'Acme', hasPasskey: true }),
      loginMeta({ id: 'plain', title: 'Basecamp' })
    ])
    render(<ListColumn />)

    const marks = screen.getAllByTitle('Passkey')
    expect(marks).toHaveLength(1)
    expect(marks[0].closest('[data-testid="entry-item"]')).toHaveTextContent('Acme')
  })
})

// The app's only search field lives in this column, so its accelerators are
// asserted against the same rows they act on.
describe('List search', () => {
  const field = () => screen.getByTestId('search-input')

  beforeEach(() => vi.clearAllMocks())

  it('narrows the list as you type and restores it on clear', async () => {
    seed()
    render(<ListColumn />)

    await userEvent.type(field(), 'air')
    expect(screen.getByText('Airbnb')).toBeInTheDocument()
    expect(screen.queryByText('Zebra')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('search-clear-button'))
    expect(titles()).toEqual(['Zebra', 'Airbnb', 'Monzo', 'Basecamp'])
  })

  // A query is answered by relevance, not by the sort control: re-sorting the
  // ranked results buried the closest match under whatever was newest.
  it('keeps the search ranking, so the best match leads', async () => {
    withEntries([
      loginMeta({ id: 'loose', title: 'Monzo Business Account', updatedAt: at(2024, 2, 14, 9) }),
      loginMeta({ id: 'exact', title: 'Monzo', updatedAt: at(2024, 0, 12, 12) })
    ])
    render(<ListColumn />)

    // Recency alone would lead with the newer, looser match.
    expect(titles()).toEqual(['Monzo Business Account', 'Monzo'])

    await userEvent.type(field(), 'monzo')
    expect(titles()).toEqual(['Monzo', 'Monzo Business Account'])
  })

  it('selects the first visible row on ⏎', async () => {
    seed()
    render(<ListColumn />)

    // No query: the first row of the list as sorted (newest first).
    await userEvent.type(field(), '{Enter}')
    expect(useVault.getState().currentId).toBe('fresh')

    // With one: the first row the query leaves standing.
    await userEvent.type(field(), 'air{Enter}')
    expect(useVault.getState().currentId).toBe('yday')
  })

  it('copies the first visible row’s primary secret on ⌘⏎', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'fresh', password: 's3cret' }))
    seed()
    render(<ListColumn />)

    await userEvent.click(field())
    await userEvent.keyboard('{Meta>}{Enter}{/Meta}')

    expect(calls('reveal_entry')).toContainEqual({ id: 'fresh' })
    await vi.waitFor(() =>
      expect(calls('copy_to_clipboard')).toContainEqual(
        { value: 's3cret', clearAfterMs: expect.anything() }
      )
    )
    // Copying is not selecting.
    expect(useVault.getState().currentId).toBeNull()
  })

  it('clears the query on Esc, then blurs', async () => {
    seed()
    render(<ListColumn />)

    await userEvent.type(field(), 'air')
    await userEvent.keyboard('{Escape}')
    expect(field()).toHaveValue('')
    expect(field()).toHaveFocus()

    await userEvent.keyboard('{Escape}')
    expect(field()).not.toHaveFocus()
  })
})

// The arrows belong to the whole column, so they are asserted from both ends of
// it: with the caret in the search field, and with a row itself focused.
describe('List keyboard navigation', () => {
  const field = () => screen.getByTestId('search-input')
  const currentId = () => useVault.getState().currentId

  beforeEach(() => vi.clearAllMocks())

  it('walks the visible rows with ↓/↑ without taking the caret out of the field', async () => {
    seed()
    render(<ListColumn />)
    await userEvent.click(field())

    // Nothing selected yet: ↓ opens the list at its top row.
    await userEvent.keyboard('{ArrowDown}')
    expect(currentId()).toBe('fresh')

    await userEvent.keyboard('{ArrowDown}{ArrowDown}')
    expect(currentId()).toBe('week')
    await userEvent.keyboard('{ArrowUp}')
    expect(currentId()).toBe('yday')

    expect(field()).toHaveFocus()
  })

  it('clamps at both ends rather than wrapping', async () => {
    seed()
    render(<ListColumn />)
    await userEvent.click(field())

    // One press more than there are rows, at each end.
    await userEvent.keyboard('{ArrowDown>5/}')
    expect(currentId()).toBe('old')
    await userEvent.keyboard('{ArrowUp>5/}')
    expect(currentId()).toBe('fresh')
  })

  it('moves on from the selected row, carrying focus when the row has it', async () => {
    seed()
    render(<ListColumn />)

    await userEvent.click(screen.getByText('Monzo'))
    expect(currentId()).toBe('week')

    await userEvent.keyboard('{ArrowDown}')
    expect(currentId()).toBe('old')
    // A focused row hands focus to the row the arrows land on.
    expect(screen.getAllByTestId('entry-item')[3]).toHaveFocus()
  })

  it('re-aims at the first row left standing when a query hides the selection', async () => {
    seed()
    render(<ListColumn />)

    await userEvent.click(screen.getByText('Monzo'))
    await userEvent.type(field(), 'air')
    await userEvent.keyboard('{ArrowDown}')

    expect(currentId()).toBe('yday')
  })

  it('points ⌘⏎ at the row the arrows landed on', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'yday', password: 'airbnb' }))
    seed()
    render(<ListColumn />)

    await userEvent.click(field())
    await userEvent.keyboard('{ArrowDown}{ArrowDown}')
    await userEvent.keyboard('{Meta>}{Enter}{/Meta}')

    // The second row, not the first the empty query would have offered.
    expect(calls('reveal_entry')).toContainEqual({ id: 'yday' })
    await vi.waitFor(() =>
      expect(calls('copy_to_clipboard')).toContainEqual(
        { value: 'airbnb', clearAfterMs: expect.anything() }
      )
    )
  })

  it('leaves ⏎ on the column’s own controls to that control', async () => {
    seed()
    render(<ListColumn actions={<SortMenu />} />)

    // The sort button opens its menu on ⏎; selecting a row as well would be
    // two actions on one press.
    screen.getByTestId('sort-menu').focus()
    await userEvent.keyboard('{Enter}')

    expect(screen.getByTestId('sort-option-recent')).toBeInTheDocument()
    expect(useVault.getState().currentId).toBeNull()
  })

  it('leaves the arrows alone while the sort menu owns them', async () => {
    seed()
    render(<ListColumn actions={<SortMenu />} />)

    await userEvent.click(screen.getByTestId('sort-menu'))
    await userEvent.keyboard('{ArrowDown}{ArrowDown}')

    expect(useVault.getState().currentId).toBeNull()
  })

  it('leaves the arrows alone in fields outside the column', async () => {
    seed()
    render(
      <>
        <input data-testid="outside-field" />
        <ListColumn />
      </>
    )

    await userEvent.click(screen.getByTestId('outside-field'))
    await userEvent.keyboard('{ArrowDown}{ArrowUp}')

    expect(useVault.getState().currentId).toBeNull()
  })
})

// A mixed list is sectioned by kind, in registry order, keeping the sort
// order inside each section. Not a preference: it is what a mixed list is.
describe('Grouped by kind', () => {
  afterEach(() => setView('items'))

  const mixed = [
    loginMeta({ id: 'g', title: 'Google', updatedAt: at(2024, 2, 14, 9) }),
    loginMeta({ id: 'v', type: 'card', title: 'Visa', urlHost: '', favorite: true, updatedAt: at(2024, 2, 14, 10) }),
    loginMeta({ id: 'a', title: 'Airbnb', favorite: true, updatedAt: at(2024, 2, 13, 18) }),
    loginMeta({ id: 'j', type: 'note', title: 'Journal', urlHost: '', updatedAt: at(2024, 2, 14, 11) })
  ]

  it('sections the list by kind in registry order, sorted within each', () => {
    withEntries(mixed)
    render(<ListColumn />)

    // Logins, then cards, then notes — and inside Logins the starred row is
    // still pinned over the newer one, exactly as in the flat recent list.
    expect(titles()).toEqual(['Airbnb', 'Google', 'Visa', 'Journal'])
    expect(screen.getByTestId('group-login')).toHaveTextContent('Logins2')
    expect(screen.getByTestId('group-card')).toHaveTextContent('Credit cards1')
    expect(screen.getByTestId('group-note')).toHaveTextContent('Secure notes1')
  })

  it('opens the kind from its caption in All Items', async () => {
    withEntries(mixed)
    render(<ListColumn />)

    await userEvent.click(screen.getByTestId('group-card'))

    expect(useUi.getState()).toMatchObject({ view: 'items', filterType: 'card' })
    expect(titles()).toEqual(['Visa'])
    // One kind is one section, so the captions go.
    expect(screen.queryByTestId('group-card')).not.toBeInTheDocument()
  })

  it('captions the other views without opening from them', () => {
    withEntries(mixed)
    setView('favorites')
    render(<ListColumn />)

    // Two starred kinds, so two sections — and their captions are text.
    expect(titles()).toEqual(['Airbnb', 'Visa'])
    const caption = screen.getByTestId('group-login')
    expect(caption).toHaveTextContent('Logins1')
    expect(caption.tagName).toBe('DIV')
  })

  it('drops the sections while a query is ranking the rows', async () => {
    withEntries(mixed)
    render(<ListColumn />)

    await userEvent.type(screen.getByPlaceholderText('Search'), 'a')
    expect(screen.queryByTestId('group-login')).not.toBeInTheDocument()
  })

  it('draws no caption over a list of one kind', () => {
    withEntries(entries)
    render(<ListColumn />)

    expect(screen.queryByTestId('group-login')).not.toBeInTheDocument()
    expect(titles()).toEqual(['Zebra', 'Airbnb', 'Monzo', 'Basecamp'])
  })
})
