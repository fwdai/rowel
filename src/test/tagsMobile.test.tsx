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

// The chips in the field, and the chips floating under it, by name.
const applied = () =>
  within(screen.getByTestId('tag-picker-field'))
    .queryAllByRole('button', { name: /^Remove tag / })
    .map(chip => chip.textContent)
const suggested = () =>
  screen.queryAllByTestId(/^tag-suggest-/).map(chip => chip.textContent)
const field = () => screen.getByTestId('tag-picker-input')

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
    // Tinted in a hue of its own, hashed from the name, like a workspace tile.
    expect(chip).toHaveClass('monogram')
    expect(chip.style.getPropertyValue('--hue')).not.toBe('')
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

  it('opens a sheet with the entry’s tags in the field and the rest floating under it', async () => {
    seed()
    const sheet = await openPicker()

    expect(sheet).toHaveAttribute('data-frame', 'bottom-sheet')
    expect(screen.getByRole('heading', { name: 'Tags' })).toBeInTheDocument()
    expect(sheet).toHaveTextContent('Google')
    expect(applied()).toEqual(['work'])
    // Busiest first, with the count, and never a tag already in the field.
    expect(suggested()).toEqual(['money1', 'travel1'])
    expect(field()).toHaveFocus()
    // No card around either: chips are the content.
    expect(sheet.querySelector('.shadow-card')).toBeNull()
    // Nothing is opened, and nothing is written, by opening the sheet.
    expect(useVault.getState().editing).toBe(false)
    expect(calls('save_entry')).toHaveLength(0)
  })

  it('files the entry under a new tag from the read screen, without the editor', async () => {
    seed()
    await openPicker()

    await userEvent.type(field(), 'legacy')
    // Nothing is called that, so a dashed chip offers to make it; the rest
    // are narrowed to nothing.
    expect(screen.getByTestId('tag-picker-create')).toHaveTextContent('Create “legacy”')
    expect(suggested()).toEqual([])

    await userEvent.keyboard('{Enter}')

    // Made, in the field after the others, and the caret cleared for the next.
    expect(applied()).toEqual(['work', 'legacy'])
    expect(field()).toHaveValue('')
    expect(field()).toHaveFocus()
    expect(screen.queryByTestId('tag-picker-create')).not.toBeInTheDocument()
    expect(suggested()).toEqual(['money1', 'travel1'])
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

  it('moves a tag between the field and the floating chips, and saves once on the way out', async () => {
    seed()
    await openPicker()

    // Off the entry: the chip leaves the field and floats with the others.
    await userEvent.click(screen.getByLabelText('Remove tag work'))
    expect(applied()).toEqual([])
    expect(suggested()).toEqual(['work2', 'money1', 'travel1'])

    // Onto it: a floating chip tapped goes into the field.
    await userEvent.click(screen.getByTestId('tag-suggest-money'))
    expect(applied()).toEqual(['money'])
    expect(suggested()).toEqual(['work2', 'travel1'])
    expect(field()).toHaveFocus()
    // In the field it wears the same hue it has on the entry; floating, it
    // was a bare outline.
    expect(screen.getByLabelText('Remove tag money')).toHaveClass('monogram')
    expect(screen.getByTestId('tag-suggest-work')).not.toHaveClass('monogram')

    await closePicker()

    await waitFor(() => expect(calls('save_entry')).toHaveLength(1))
    expect(calls('save_entry')[0]).toEqual({
      entry: expect.objectContaining({ id: 'l1', tags: ['money'] })
    })
  })

  it('takes the last tag off with Backspace in the empty field', async () => {
    seed()
    await openPicker()

    await userEvent.click(screen.getByTestId('tag-suggest-money'))
    await userEvent.keyboard('{Backspace}')
    expect(applied()).toEqual(['work'])

    // Only the empty field does: a Backspace through typed text is editing.
    await userEvent.type(field(), 'x{Backspace}')
    expect(applied()).toEqual(['work'])
  })

  it('writes nothing when the sheet is closed as it was opened', async () => {
    seed()
    await openPicker()

    await userEvent.click(screen.getByTestId('tag-suggest-money'))
    await userEvent.click(screen.getByLabelText('Remove tag money'))
    await closePicker()

    await waitFor(() => expect(screen.queryByTestId('tag-picker')).not.toBeInTheDocument())
    expect(calls('save_entry')).toHaveLength(0)
  })

  it('treats a name that only differs by case as the tag already there', async () => {
    seed()
    await openPicker()

    await userEvent.type(field(), 'MONEY')
    expect(screen.queryByTestId('tag-picker-create')).not.toBeInTheDocument()
    expect(suggested()).toEqual(['money1'])

    await userEvent.keyboard('{Enter}')
    expect(applied()).toEqual(['work', 'money'])
    expect(field()).toHaveValue('')
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
    expect(suggested()).toEqual([])
  })

  it('reports a save the backend refused, and keeps the entry where it was', async () => {
    mockCommand('save_entry', () => {
      throw new Error('locked')
    })
    seed()
    await openPicker()

    await userEvent.click(screen.getByTestId('tag-suggest-money'))
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
    await userEvent.click(await screen.findByTestId('tag-suggest-money'))
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
