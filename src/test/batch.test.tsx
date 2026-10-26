import { describe, it, expect, beforeEach } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent, { type UserEvent } from '@testing-library/user-event'
import Body from '@/components/Main/Body'
import NoticeToast from '@/components/elements/NoticeToast'
import { useVault, useUi, setView, setFilterQuery } from '@/store'
import { withEntries, loginMeta, deletedMeta } from './utils'
import { calls, mockCommand } from './ipc'

// Four rows, alphabetical so the list order is the order written here.
const entries = ['Alpha', 'Bravo', 'Charlie', 'Delta'].map((title, i) =>
  loginMeta({ id: title.toLowerCase(), title, urlHost: '', updatedAt: `2024-01-0${4 - i}T00:00:00.000Z` })
)

let user: UserEvent

const row = (title: string) => screen.getByText(title).closest('[role="option"]') as HTMLElement
const selected = () =>
  screen
    .getAllByRole('option')
    .filter(option => option.getAttribute('aria-selected') === 'true')
    .map(option => option.textContent?.match(/Alpha|Bravo|Charlie|Delta/)?.[0])

const withKey = async (key: string, act: () => Promise<void>) => {
  await user.keyboard(`{${key}>}`)
  await act()
  await user.keyboard(`{/${key}}`)
}

const cmdClick = (title: string) => withKey('Meta', () => user.click(row(title)))
const shiftClick = (title: string) => withKey('Shift', () => user.click(row(title)))

beforeEach(() => {
  user = userEvent.setup()
  withEntries(entries)
  useVault.setState({ currentId: null })
})

const renderBody = () =>
  render(
    <>
      <Body />
      <NoticeToast />
    </>
  )

describe('picking a batch', () => {
  it('⌘-click adds the open row and the clicked one, and the bar and summary come up', async () => {
    renderBody()
    await user.click(row('Alpha'))
    expect(screen.queryByTestId('batch-bar')).not.toBeInTheDocument()

    await cmdClick('Charlie')

    expect(selected()).toEqual(['Alpha', 'Charlie'])
    expect(screen.getByTestId('batch-count')).toHaveTextContent('2 selected')
    expect(screen.getByTestId('batch-summary')).toHaveTextContent('2 items selected')
  })

  it('⌘-click on a picked row takes it out, and one left is a plain selection', async () => {
    renderBody()
    await user.click(row('Alpha'))
    await cmdClick('Bravo')
    await cmdClick('Alpha')

    expect(selected()).toEqual(['Bravo'])
    expect(useVault.getState().currentId).toBe('bravo')
    expect(screen.queryByTestId('batch-bar')).not.toBeInTheDocument()
  })

  it('⇧-click picks the run from the anchor, and a second one re-draws it from there', async () => {
    renderBody()
    await user.click(row('Bravo'))
    await shiftClick('Delta')
    expect(selected()).toEqual(['Bravo', 'Charlie', 'Delta'])

    await shiftClick('Alpha')
    expect(selected()).toEqual(['Alpha', 'Bravo'])
  })

  it('⇧↓ grows the batch from the keyboard', async () => {
    renderBody()
    await user.click(row('Alpha'))
    await user.keyboard('{Shift>}{ArrowDown}{ArrowDown}{/Shift}')

    expect(selected()).toEqual(['Alpha', 'Bravo', 'Charlie'])
  })

  it('⌘A on a row picks every row the list shows', async () => {
    renderBody()
    await user.click(row('Bravo'))
    await user.keyboard('{Meta>}a{/Meta}')

    expect(selected()).toEqual(['Alpha', 'Bravo', 'Charlie', 'Delta'])
  })

  it('a plain click ends the batch on the row clicked', async () => {
    renderBody()
    await user.click(row('Alpha'))
    await cmdClick('Bravo')
    await user.click(row('Delta'))

    expect(selected()).toEqual(['Delta'])
    expect(screen.queryByTestId('batch-bar')).not.toBeInTheDocument()
  })

  it('Esc ends the batch on the row the keyboard was on', async () => {
    renderBody()
    await user.click(row('Alpha'))
    await cmdClick('Charlie')
    await user.keyboard('{Escape}')

    expect(selected()).toEqual(['Charlie'])
    expect(useVault.getState().picked.size).toBe(0)
  })

  it('a new query ends the batch', async () => {
    renderBody()
    await user.click(row('Alpha'))
    await cmdClick('Bravo')

    setFilterQuery('a')

    await waitFor(() => expect(screen.queryByTestId('batch-bar')).not.toBeInTheDocument())
  })
})

describe('acting on a batch', () => {
  it('archives every picked row on the second press', async () => {
    renderBody()
    await user.click(row('Alpha'))
    await cmdClick('Charlie')

    await user.click(screen.getByTestId('batch-archive'))
    expect(calls('delete_entry')).toHaveLength(0)
    expect(screen.getByTestId('batch-count')).toHaveTextContent('Archive 2 items?')

    await user.click(screen.getByTestId('batch-archive-confirm'))

    await waitFor(() => expect(useVault.getState().items.map(e => e.id)).toEqual(['bravo', 'delta']))
    expect(calls('delete_entry').map(call => call.id).sort()).toEqual(['alpha', 'charlie'])
    expect(screen.queryByTestId('batch-bar')).not.toBeInTheDocument()
  })

  it('arms with ⌘⌫ and backs out with Esc before letting the batch go', async () => {
    renderBody()
    await user.click(row('Alpha'))
    await cmdClick('Bravo')

    await user.keyboard('{Meta>}{Backspace}{/Meta}')
    expect(screen.getByTestId('batch-archive-confirm')).toBeInTheDocument()

    await user.keyboard('{Escape}')
    expect(screen.getByTestId('batch-archive')).toBeInTheDocument()
    expect(selected()).toEqual(['Alpha', 'Bravo'])
  })

  it('stars the batch, and unstars it once every row wears one', async () => {
    mockCommand('set_favorite', ({ id, favorite }) => ({ ...entries.find(e => e.id === id), favorite }))
    renderBody()
    await user.click(row('Alpha'))
    await cmdClick('Bravo')

    await user.click(screen.getByTestId('batch-favorite'))
    await waitFor(() =>
      expect(useVault.getState().items.filter(e => e.favorite).map(e => e.id)).toEqual(['alpha', 'bravo'])
    )
    expect(calls('set_favorite').every(call => call.favorite === true)).toBe(true)
    // Still picked: starring takes nothing out of All Items.
    expect(selected()).toEqual(['Alpha', 'Bravo'])

    await user.click(screen.getByTestId('batch-favorite'))
    await waitFor(() => expect(useVault.getState().items.some(e => e.favorite)).toBe(false))
  })

  it('keeps what landed when part of a batch fails, and says so', async () => {
    mockCommand('delete_entry', ({ id }) => (id === 'bravo' ? Promise.reject(new Error('locked')) : undefined))
    renderBody()
    await user.click(row('Alpha'))
    await cmdClick('Bravo')
    await user.click(screen.getByTestId('batch-archive'))
    await user.click(screen.getByTestId('batch-archive-confirm'))

    await waitFor(() => expect(useVault.getState().items.map(e => e.id)).toEqual(['bravo', 'charlie', 'delta']))
    expect(await screen.findByTestId('notice-toast')).toHaveTextContent('Some items could not be changed')
  })

  it('restores or deletes for good in the Archive', async () => {
    const tombstones = ['x', 'y', 'z'].map(id => deletedMeta({ id, title: `Gone ${id}` }))
    mockCommand('list_deleted', () => tombstones)
    renderBody()
    setView('archive')
    await waitFor(() => expect(useVault.getState().archive).toHaveLength(3))

    await user.click(row('Gone x'))
    await cmdClick('Gone y')
    expect(screen.queryByTestId('batch-favorite')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('batch-restore'))

    await waitFor(() => expect(useVault.getState().archive.map(e => e.id)).toEqual(['z']))
    expect(calls('restore_entry').map(call => call.id).sort()).toEqual(['x', 'y'])
    expect(useUi.getState().view).toBe('archive')
  })
})
