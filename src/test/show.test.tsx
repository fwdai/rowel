import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Show from '@/components/Main/Body/Aside/Show'
import Aside from '@/components/Main/Body/Aside'
import Body from '@/components/Main/Body'
import type { Entry } from '@/api/types'
import { setCurrentEntry, useUi, useVault } from '@/store'
import { kindOf } from '@/kinds'
import { t } from '@/i18n'
import { withEntries, loginEntry, loginMeta, deferred } from './utils'
import { calls, mockCommand } from './ipc'

beforeEach(() => vi.clearAllMocks())

// The detail pane's chrome: what it says about the entry around the kind's own
// field set. The field set itself is covered by entry.test.tsx.
describe('Show chrome', () => {
  it('names the kind once and reduces the stamps to one footer line', async () => {
    mockCommand('reveal_entry', () => loginEntry())
    render(<Show entry={loginMeta()} />)

    // The eyebrow above the title is the only place the kind is named; the
    // "Type" ledger cell that repeated it is gone.
    expect(await screen.findAllByText(t(kindOf('login').label))).toHaveLength(1)
    expect(screen.queryByText('Type')).not.toBeInTheDocument()
    // The footer is labelled cells, not a sentence: a label over each stamp.
    const footer = await screen.findByTestId('entry-footer')
    expect(footer).toHaveTextContent('Modified')
    expect(footer).toHaveTextContent('Created')
  })

  // A rejected reveal used to be swallowed, leaving a pane that looked like one
  // still loading — no rows, no message, and in edit mode no way out.
  it('says so when the reveal fails, and offers a retry and a way out', async () => {
    mockCommand('reveal_entry', () => Promise.reject({ kind: 'other', message: 'refused' }))
    withEntries([loginMeta()])
    setCurrentEntry('l1')
    render(<Show entry={loginMeta()} editing />)

    const error = await screen.findByTestId('reveal-error')
    expect(error).toHaveTextContent('Could not open this entry. Please try again.')
    expect(screen.queryByTestId('entry-sheet')).not.toBeInTheDocument()

    mockCommand('reveal_entry', () => loginEntry())
    await userEvent.click(screen.getByTestId('reveal-retry-button'))
    expect(await screen.findByTestId('entry-sheet')).toBeInTheDocument()
  })

  // The failure is the entry's, not the pane's: selecting another row must not
  // flash the previous row's error under the new title.
  it('keeps one entry’s failure off the next entry', async () => {
    mockCommand('reveal_entry', ({ id }) =>
      id === 'l1' ? Promise.reject({ kind: 'other', message: 'refused' }) : new Promise(() => {})
    )
    withEntries([loginMeta(), loginMeta({ id: 'l2', title: 'Other' })])
    const { rerender } = render(<Show entry={loginMeta()} />)
    await screen.findByTestId('reveal-error')

    rerender(<Show entry={loginMeta({ id: 'l2', title: 'Other' })} />)

    expect(screen.queryByTestId('reveal-error')).not.toBeInTheDocument()
  })

  it('keeps the last entry up, inert, until the next one is revealed', async () => {
    const next = deferred<Entry>()
    mockCommand('reveal_entry', ({ id }) => (id === 'l1' ? loginEntry() : next.promise))
    withEntries([loginMeta(), loginMeta({ id: 'l2', title: 'Other' })])
    setCurrentEntry('l1')
    const { rerender } = render(<Show entry={loginMeta()} />)
    await screen.findByTestId('entry-footer')

    setCurrentEntry('l2')
    rerender(<Show entry={loginMeta({ id: 'l2', title: 'Other' })} />)

    expect(screen.getByRole('heading', { name: 'Google' })).toBeInTheDocument()
    expect(screen.getByTestId('entry-footer')).toBeInTheDocument()
    expect(screen.getByTestId('primary-action-button')).toBeEnabled()
    expect(screen.getByTestId('primary-action-button').closest('[inert]')).not.toBeNull()
    await userEvent.keyboard('{Enter}')
    expect(calls('copy_to_clipboard')).toEqual([])

    next.resolve(loginEntry({ id: 'l2', title: 'Other' }))
    expect(await screen.findByRole('heading', { name: 'Other' })).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Google' })).not.toBeInTheDocument()
    expect(screen.getByTestId('primary-action-button').closest('[inert]')).toBeNull()
  })

  it('keeps an entry’s rows up while it is revealed again', async () => {
    let reveal: () => unknown = () => loginEntry()
    mockCommand('reveal_entry', () => reveal())
    const { rerender } = render(<Show entry={loginMeta()} />)
    const footer = await screen.findByTestId('entry-footer')

    reveal = () => new Promise(() => {})
    rerender(<Show entry={loginMeta({ updatedAt: '2024-02-01T00:00:00.000Z' })} />)
    await waitFor(() => expect(calls('reveal_entry')).toHaveLength(2))

    expect(screen.getByTestId('entry-footer')).toBe(footer)
    expect(screen.getByTestId('attach-file-button')).toBeInTheDocument()
    await userEvent.keyboard('{Enter}')
    expect(calls('copy_to_clipboard')).toEqual([])
  })

  it('leaves a failed reveal’s way out pressable after another entry was up', async () => {
    mockCommand('reveal_entry', ({ id }) =>
      id === 'l1' ? loginEntry() : Promise.reject({ kind: 'other', message: 'refused' })
    )
    withEntries([loginMeta(), loginMeta({ id: 'l2', title: 'Other' })])
    setCurrentEntry('l1')
    const { rerender } = render(<Show entry={loginMeta()} />)
    await screen.findByTestId('entry-footer')

    setCurrentEntry('l2')
    rerender(<Show entry={loginMeta({ id: 'l2', title: 'Other' })} />)

    const retry = await screen.findByTestId('reveal-retry-button')
    expect(retry.closest('[inert]')).toBeNull()
    expect(screen.getByTestId('reveal-close-button').closest('[inert]')).toBeNull()
    await userEvent.click(screen.getByTestId('reveal-close-button'))
    expect(useVault.getState().currentId).toBeNull()
  })

  it('closes the failed entry from the error surface', async () => {
    mockCommand('reveal_entry', () => Promise.reject({ kind: 'other', message: 'refused' }))
    withEntries([loginMeta()])
    setCurrentEntry('l1')
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('reveal-close-button'))
    expect(useVault.getState().currentId).toBeNull()
  })

  it('filters the list by a tag pressed in the detail pane', async () => {
    mockCommand('reveal_entry', () => loginEntry({ tags: ['work'] }))
    render(<Show entry={loginMeta({ tags: ['work'] })} />)

    await userEvent.click(await screen.findByLabelText('Filter by tag work'))
    expect(useUi.getState().query).toBe('work')
  })
})

// Entering edit mode: the pane swaps faces in place, and the list column steps
// back while it does.
describe('Edit mode in the pane', () => {
  const seed = () => {
    withEntries([loginMeta()])
    setCurrentEntry('l1')
  }

  beforeEach(() => mockCommand('reveal_entry', () => loginEntry()))

  // Edit lives under the header's one menu, not as a button of its own.
  const openEdit = async () => {
    await userEvent.click(await screen.findByTestId('more-actions-button'))
    await userEvent.click(screen.getByTestId('edit-entry-button'))
  }

  it('replaces the read view with the editor, in the same pane', async () => {
    seed()
    render(<Aside />)

    expect(screen.queryByTestId('entry-sheet')).not.toBeInTheDocument()
    expect(screen.queryByTestId('edit-entry-button')).not.toBeInTheDocument()
    await openEdit()

    expect(useVault.getState().editing).toBe(true)
    expect(screen.getByTestId('entry-sheet')).toBeInTheDocument()
    // The read cluster goes with it: one set of actions at a time.
    expect(screen.queryByTestId('primary-action-button')).not.toBeInTheDocument()
    expect(screen.getByTestId('save-entry-button')).toBeInTheDocument()
  })

  // The footer's tags cell is never a hole: with nothing filed, it is the way
  // to file something, and that goes straight to the editor.
  it('opens the editor from the empty tags cell', async () => {
    seed()
    render(<Aside />)

    expect(screen.queryByLabelText(/Filter by tag/)).not.toBeInTheDocument()
    await userEvent.click(await screen.findByTestId('add-tag-button'))

    expect(useVault.getState().editing).toBe(true)
    expect(screen.getByTestId('tags-input')).toBeInTheDocument()
  })

  it('leaves the list column visible but quiet and inert while writing', async () => {
    seed()
    render(<Body />)
    expect(screen.getByTestId('list-column')).not.toHaveClass('opacity-60')

    await openEdit()
    expect(screen.getByTestId('list-column')).toHaveClass('opacity-60')
    // `inert`, not `pointer-events-none`: the keyboard has to stand down too.
    expect(screen.getByTestId('list-column')).toHaveAttribute('inert')
  })
})
