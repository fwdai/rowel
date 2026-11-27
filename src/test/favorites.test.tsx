import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Sidebar from '@/components/Main/Sidebar'
import ListColumn from '@/components/Main/Body/ListColumn'
import SortMenu from '@/components/Main/Body/List/SortMenu'
import Body from '@/components/Main/Body'
import Show from '@/components/Main/Body/Aside/Show'
import { useVault, selectCurrent, setView, setCurrentEntry } from '@/store'
import { withEntries, loginEntry, loginMeta } from './utils'
import { calls, mockCommand } from './ipc'

const starred = loginMeta({ id: 'star', title: 'Monzo', favorite: true })
const plain = loginMeta({ id: 'plain', title: 'Airbnb' })

const titles = () => screen.getAllByTestId('entry-item-title').map(el => el.textContent)

const seed = () => withEntries([plain, starred])

beforeEach(() => vi.clearAllMocks())

describe('the favorite toggle', () => {
  it('stars an unstarred entry and keeps the new value in the list', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'plain' }))
    seed()
    render(<Show entry={plain} />)

    await userEvent.click(await screen.findByTestId('favorite-toggle'))

    expect(calls('set_favorite')).toContainEqual({ id: 'plain', favorite: true })
    await vi.waitFor(() =>
      expect(useVault.getState().items.find(e => e.id === 'plain')?.favorite).toBe(true)
    )
  })

  it('unstars an entry that is already starred', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'star' }))
    seed()
    render(<Show entry={starred} />)

    await userEvent.click(await screen.findByTestId('favorite-toggle'))

    expect(calls('set_favorite')).toContainEqual({ id: 'star', favorite: false })
  })

  it('is absent for an archived entry — a tombstone cannot be starred', () => {
    render(<Show entry={loginMeta({ deletedAt: '2024-02-01T00:00:00.000Z' })} />)
    expect(screen.queryByTestId('favorite-toggle')).not.toBeInTheDocument()
  })
})

describe('the Favorites view', () => {
  it('sits between All Items and the Archive in the rail', () => {
    render(<Sidebar />)

    const rail = screen.getAllByRole('button').map(b => b.getAttribute('data-testid'))
    expect(rail).toEqual([
      'add-entry-button',
      'view-items',
      'view-favorites',
      'view-archive',
      'tags-button',
      'lock-vault-button',
      'settings-button'
    ])
  })

  it('lists only the starred entries and titles the column after itself', () => {
    seed()
    setView('favorites')
    render(<ListColumn />)

    expect(titles()).toEqual(['Monzo'])
    expect(screen.getByTestId('list-title')).toHaveTextContent('Favorites')
  })

  it('marks starred rows with a star in every view', () => {
    seed()
    render(<ListColumn />)

    expect(screen.getAllByTestId('entry-item-star')).toHaveLength(1)
  })

  it('pins favorites above the rest under recency, and leaves alphabetical alone', async () => {
    // `plain` is the more recently touched of the two, so recency alone would
    // put it first: only the pin can float the starred row over it.
    withEntries([
      loginMeta({ id: 'plain', title: 'Airbnb', updatedAt: '2024-03-01T00:00:00.000Z' }),
      loginMeta({ id: 'star', title: 'Monzo', favorite: true, updatedAt: '2024-01-01T00:00:00.000Z' })
    ])
    render(<ListColumn actions={<SortMenu />} />)

    expect(titles()).toEqual(['Monzo', 'Airbnb'])

    await userEvent.click(screen.getByTestId('sort-menu'))
    await userEvent.click(screen.getByText('Name (A–Z)'))
    expect(titles()).toEqual(['Airbnb', 'Monzo'])
  })

  it('says how to fill itself when nothing is starred', () => {
    withEntries([plain])
    setView('favorites')
    render(<Body />)

    expect(screen.getByTestId('empty-favorites')).toBeInTheDocument()
    expect(screen.getByText('Star an entry to keep it here.')).toBeInTheDocument()
    expect(screen.queryByTestId('entry-item')).not.toBeInTheDocument()
  })

  it('drops the selection when the shown entry is unstarred from inside it', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'star' }))
    withEntries([starred])
    setView('favorites')
    setCurrentEntry('star')
    render(<Body />)

    await userEvent.click(await screen.findByTestId('favorite-toggle'))

    await vi.waitFor(() => expect(selectCurrent(useVault.getState())).toBeNull())
    expect(screen.getByTestId('empty-favorites')).toBeInTheDocument()
  })
})
