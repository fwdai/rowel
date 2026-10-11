import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import { useUi, useVault } from '@/store'
import { withEntries, loginEntry, loginMeta } from './utils'
import { calls, mockCommand } from './ipc'
import { setLayout } from './layout'

// The phone's tags: one chip wherever a tag is drawn, and a picker sheet
// instead of the editor for filing an entry under one. The whole shell is
// rendered, as `compact.test` does, so the sheet is framed the way `Main`
// frames it there.
const seed = () =>
  withEntries([
    loginMeta({ id: 'l1', title: 'Google', tags: ['work'] }),
    loginMeta({ id: 'l2', title: 'Airbnb', tags: ['work', 'travel'] }),
    loginMeta({ id: 'l3', title: 'Monzo', tags: ['money'] })
  ])

const openEntry = async () => {
  render(<Main />)
  await userEvent.click(screen.getByText('Google'))
  return screen.findByTestId('tags-section')
}

const openPicker = async () => {
  await openEntry()
  await userEvent.click(screen.getByTestId('add-tag-button'))
  return screen.findByTestId('tag-picker')
}

// Backing out the way a screen reader does; the scrim and the grabber are the
// finger's ways.
const closePicker = () => userEvent.click(screen.getByTestId('modal-close'))

const checked = (tag: string) => screen.getByTestId(`tag-pick-${tag}`).getAttribute('aria-checked')

beforeEach(() => {
  vi.clearAllMocks()
  setLayout('compact')
  mockCommand('reveal_entry', ({ id }) =>
    loginEntry({ id: id as string, title: 'Google', tags: ['work'] })
  )
  mockCommand('save_entry', ({ entry }) => {
    const { id, title, tags } = entry as { id: string; title: string; tags: string[] }
    return loginMeta({ id, title, tags, updatedAt: '2024-02-01T00:00:00.000Z' })
  })
})

describe('tags on the entry screen', () => {
  it('draws them as chips with no card, and the way to add one among them', async () => {
    seed()
    const section = await openEntry()

    const chip = within(section).getByLabelText('Filter by tag work')
    expect(chip).toHaveTextContent('work')
    expect(chip).not.toHaveTextContent('#')
    // No card under the chips: they are the content.
    expect(section.querySelector('.shadow-card')).toBeNull()
    expect(within(section).getByTestId('add-tag-button')).toHaveTextContent('Add tag')
  })

  it('goes to everything filed under a pressed tag, named without a hash', async () => {
    seed()
    await openEntry()

    await userEvent.click(screen.getByLabelText('Filter by tag work'))

    expect(useUi.getState().view).toBe('tags')
    expect(useUi.getState().filterTag).toBe('work')
    expect(screen.getByTestId('list-title')).toHaveTextContent(/^work$/)
    expect(screen.getByTestId('active-tag')).toHaveTextContent(/^work$/)
  })

  it('offers the vault’s tags in a sheet, busiest first, with the entry’s checked', async () => {
    seed()
    const sheet = await openPicker()

    expect(sheet).toHaveAttribute('data-frame', 'bottom-sheet')
    expect(screen.getByRole('heading', { name: 'Tags' })).toBeInTheDocument()
    expect(sheet).toHaveTextContent('Google')
    expect(screen.getAllByRole('checkbox').map(row => row.textContent)).toEqual([
      'work2',
      'money1',
      'travel1'
    ])
    expect(checked('work')).toBe('true')
    expect(checked('money')).toBe('false')
    expect(screen.getByTestId('tag-picker-input')).toHaveFocus()
    // Nothing is opened, and nothing is written, by opening the sheet.
    expect(useVault.getState().editing).toBe(false)
    expect(calls('save_entry')).toHaveLength(0)
  })

  it('files the entry under a new tag from the read screen, without the editor', async () => {
    seed()
    await openPicker()

    await userEvent.type(screen.getByTestId('tag-picker-input'), 'legacy')
    // Nothing is called that, so the first row offers to make it.
    expect(screen.getByTestId('tag-picker-create')).toHaveTextContent('Create “legacy”')
    expect(screen.queryByTestId('tag-pick-work')).not.toBeInTheDocument()

    await userEvent.keyboard('{Enter}')

    // Made, checked, at the top, and the field cleared for the next one.
    expect(screen.getAllByRole('checkbox')[0]).toHaveTextContent(/^legacy$/)
    expect(checked('legacy')).toBe('true')
    expect(screen.getByTestId('tag-picker-input')).toHaveValue('')
    expect(screen.queryByTestId('tag-picker-create')).not.toBeInTheDocument()
    // Still nothing written: the sheet commits on the way out.
    expect(calls('save_entry')).toHaveLength(0)

    await closePicker()

    await waitFor(() => expect(calls('save_entry')).toHaveLength(1))
    // The whole entry goes back, secrets as revealed, tags as picked.
    expect(calls('save_entry')[0]).toEqual({
      entry: expect.objectContaining({
        id: 'l1',
        password: 'secret',
        username: 'me@example.com',
        tags: ['work', 'legacy']
      })
    })
    expect(useVault.getState().editing).toBe(false)
    await waitFor(() =>
      expect(screen.getByLabelText('Filter by tag legacy')).toBeInTheDocument()
    )
  })

  it('takes a tag off by unchecking it, and saves once on the way out', async () => {
    seed()
    await openPicker()

    await userEvent.click(screen.getByTestId('tag-pick-work'))
    expect(checked('work')).toBe('false')
    await userEvent.click(screen.getByTestId('tag-pick-money'))
    expect(checked('money')).toBe('true')

    await closePicker()

    await waitFor(() => expect(calls('save_entry')).toHaveLength(1))
    expect(calls('save_entry')[0]).toEqual({
      entry: expect.objectContaining({ id: 'l1', tags: ['money'] })
    })
  })

  it('writes nothing when the sheet is closed as it was opened', async () => {
    seed()
    await openPicker()

    await userEvent.click(screen.getByTestId('tag-pick-money'))
    await userEvent.click(screen.getByTestId('tag-pick-money'))
    await closePicker()

    await waitFor(() => expect(screen.queryByTestId('tag-picker')).not.toBeInTheDocument())
    expect(calls('save_entry')).toHaveLength(0)
  })

  it('treats a name that only differs by case as the tag already there', async () => {
    seed()
    await openPicker()

    await userEvent.type(screen.getByTestId('tag-picker-input'), 'MONEY')
    expect(screen.queryByTestId('tag-picker-create')).not.toBeInTheDocument()
    expect(screen.getByTestId('tag-pick-money')).toBeInTheDocument()

    await userEvent.keyboard('{Enter}')
    expect(checked('money')).toBe('true')
    expect(screen.getByTestId('tag-picker-input')).toHaveValue('')
    await closePicker()

    await waitFor(() => expect(calls('save_entry')).toHaveLength(1))
    expect(calls('save_entry')[0]).toEqual({
      entry: expect.objectContaining({ tags: ['work', 'money'] })
    })
  })

  it('says how to start when the vault carries no tags at all', async () => {
    withEntries([loginMeta({ id: 'l1', title: 'Google' })])
    mockCommand('reveal_entry', () => loginEntry({ id: 'l1', title: 'Google' }))
    await openPicker()

    expect(screen.getByTestId('tag-picker-empty')).toHaveTextContent('first tag')
    expect(screen.queryAllByRole('checkbox')).toHaveLength(0)
  })

  it('reports a save the backend refused, and keeps the entry where it was', async () => {
    mockCommand('save_entry', () => {
      throw new Error('locked')
    })
    seed()
    await openPicker()

    await userEvent.click(screen.getByTestId('tag-pick-money'))
    await closePicker()

    expect(await screen.findByRole('alert')).toHaveTextContent('Could not save')
    expect(screen.getByLabelText('Filter by tag work')).toBeInTheDocument()
    expect(screen.queryByLabelText('Filter by tag money')).not.toBeInTheDocument()
  })
})

describe('tags in the editor', () => {
  const openEdit = async () => {
    await openEntry()
    await userEvent.click(screen.getByTestId('more-actions-button'))
    await userEvent.click(screen.getByTestId('edit-entry-button'))
    expect(useVault.getState().editing).toBe(true)
  }

  it('picks into the draft and saves with the form, not before', async () => {
    seed()
    await openEdit()

    // The same chip, now with its ×, and the same sheet writing the draft.
    await userEvent.click(screen.getByTestId('add-tag-button'))
    await userEvent.click(await screen.findByTestId('tag-pick-money'))
    await closePicker()

    expect(screen.getByLabelText('Remove tag money')).toBeInTheDocument()
    expect(calls('save_entry')).toHaveLength(0)

    await userEvent.click(screen.getByLabelText('Remove tag work'))
    expect(screen.queryByLabelText('Remove tag work')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('save-entry-button'))
    await waitFor(() => expect(calls('save_entry')).toHaveLength(1))
    expect(calls('save_entry')[0]).toEqual({
      entry: expect.objectContaining({ id: 'l1', tags: ['money'] })
    })
  })

  it('does not type tags into a line on the phone', async () => {
    seed()
    await openEdit()

    expect(screen.queryByTestId('tags-input')).not.toBeInTheDocument()
    expect(screen.getByTestId('add-tag-button')).toBeInTheDocument()
  })
})
