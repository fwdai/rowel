import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import Aside from '@/components/Main/Body/Aside'
import AuditList from '@/components/Main/Body/List/Audit'
import {
  useUi,
  useVault,
  setFilterQuery,
  setFilterType,
  setView,
  setSyncStatus,
  initialApp
} from '@/store'
import { resetStores, withEntries, loginMeta } from './utils'

// One empty-state system: the variant is decided from store state, and the
// list column and the detail pane never both claim the hero.
describe('empty states', () => {
  beforeEach(() => vi.clearAllMocks())

  const seed = (entries = [loginMeta({ id: 'l1', title: 'Google' })]) => withEntries(entries)

  describe('empty vault', () => {
    it('shows the hero once, in the detail pane, and opens the kind picker', async () => {
      seed([])
      render(<Main />)

      // Exactly one — the list column stays blank rather than repeating it.
      expect(screen.getAllByText('Your vault is empty')).toHaveLength(1)
      expect(
        screen.getByText(
          'Add your first login, card or note. Everything is encrypted before it touches disk.'
        )
      ).toBeInTheDocument()
      expect(screen.queryByTestId('entry-item')).not.toBeInTheDocument()

      await userEvent.click(screen.getByTestId('create-first-entry-button'))
      expect(useUi.getState().addPicker).toBe(true)
    })

    it('offers the import route into Settings', async () => {
      seed([])
      render(<Main />)

      await userEvent.click(screen.getByText('Import from another app'))
      expect(useUi.getState().settings).toBe(true)
      expect(screen.queryByText('Restore from Google Drive')).not.toBeInTheDocument()
    })

    // A vault that syncs holds what its pack holds, so an empty one has nothing
    // up there to pull; the account's other vaults are reached from Settings.
    it('offers the same route once sync is on', () => {
      seed([])
      setSyncStatus({ ...initialApp.sync, configured: true })
      render(<Main />)

      expect(screen.getByText('Import from another app')).toBeInTheDocument()
      expect(screen.queryByText('Restore from Google Drive')).not.toBeInTheDocument()
    })
  })

  describe('kind filter with no items', () => {
    it('names the kind in the list column and starts one from the link', async () => {
      seed()
      setFilterType('card')
      render(<Main />)

      expect(screen.getByText('No credit cards yet')).toBeInTheDocument()
      // The detail pane stays on the quiet state — no second hero.
      expect(screen.getByText('Select an item')).toBeInTheDocument()
      expect(screen.queryByText('Your vault is empty')).not.toBeInTheDocument()

      await userEvent.click(screen.getByText('Add a credit card'))
      expect(useVault.getState().creating).toBe('card')
    })
  })

  describe('search with no matches', () => {
    it('quotes the query back and clears it from the link', async () => {
      seed()
      setFilterQuery('zzz')
      render(<Main />)

      expect(screen.getByText('No matches for “zzz”')).toBeInTheDocument()
      expect(screen.queryByText('Search all items')).not.toBeInTheDocument()

      await userEvent.click(screen.getByText('Clear search'))
      expect(useUi.getState().query).toBe('')
    })

    it('names the kind filter as the other half of the why, and can widen', async () => {
      seed()
      setFilterType('login')
      setFilterQuery('zzz')
      render(<Main />)

      expect(screen.getByText('No matches for “zzz” in logins')).toBeInTheDocument()

      await userEvent.click(screen.getByText('Search all items'))
      expect(useUi.getState().filterType).toBeNull()
    })
  })

  describe('nothing selected', () => {
    it('is quiet: hints, no body and no buttons', () => {
      seed()
      render(<Aside />)

      expect(screen.getByText('Select an item')).toBeInTheDocument()
      expect(screen.getByText('↑↓')).toBeInTheDocument()
      expect(screen.getByText('browse')).toBeInTheDocument()
      expect(screen.getByText('⌘K')).toBeInTheDocument()
      // The hero treatment is reserved for the empty vault.
      expect(screen.queryByRole('button')).not.toBeInTheDocument()
    })

    it('lists both copy chords, and keeps the sheet to eight', () => {
      seed()
      render(<Aside />)

      expect(screen.getByText('⌘⏎')).toBeInTheDocument()
      expect(screen.getByText('copy password')).toBeInTheDocument()
      expect(screen.getByText('⌘B')).toBeInTheDocument()
      expect(screen.getByText('copy username')).toBeInTheDocument()
      const sheet = screen.getByTestId('empty-select')
      expect(sheet.querySelectorAll('.grid > span')).toHaveLength(8)
    })
  })

  describe('vault health with nothing to audit', () => {
    const seedHealth = () => {
      withEntries([loginMeta({ id: 'l1', title: 'Google' })], {})
      setView('health')
    }

    it('explains the missing score and starts a login', async () => {
      seedHealth()
      render(<Aside />)

      expect(screen.getByText('Nothing to audit yet')).toBeInTheDocument()
      expect(
        screen.getByText('Your score appears once a login with a password is saved.')
      ).toBeInTheDocument()

      await userEvent.click(screen.getByText('Add a login'))
      expect(useVault.getState().creating).toBe('login')
    })

    it('leaves the audit column blank, and keeps the loading label until results', () => {
      seedHealth()
      const { unmount } = render(<AuditList />)
      expect(screen.queryByText('Loading Results..')).not.toBeInTheDocument()
      expect(screen.queryByTestId('entry-item')).not.toBeInTheDocument()
      unmount()

      resetStores()
      withEntries([loginMeta({ id: 'l1', title: 'Google' })])
      setView('health')
      render(<AuditList />)
      expect(screen.getByText('Loading Results..')).toBeInTheDocument()
    })
  })
})
