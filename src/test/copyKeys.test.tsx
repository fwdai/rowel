import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Body from '@/components/Main/Body'
import Main from '@/components/Main'
import { registerDialog, unregisterDialog, useUi } from '@/store'
import { withEntries, loginMeta, loginEntry } from './utils'
import { calls, clearCalls, mockCommand } from './ipc'

/*
 * The KeePass copy chords (ListColumn/copyKeys): ⌘B username, ⌘C password,
 * ⌘T one-time code, ⌘Y password and code as one string, ⌘U URL — on a
 * focused row, and the ones a text field has no use for (⌘B, ⌘T, ⌘U) from the
 * search field too. ⌘C everywhere off a row is still the system copy of
 * whatever text is selected.
 */

const entry = loginEntry({
  id: 'l1',
  username: 'ada@example.com',
  password: 'hunter2',
  website: 'https://google.com',
  otp: 'JBSWY3DPEHPK3PXP'
})

const copied = () => calls('copy_to_clipboard').map(args => args.value)

// Select the row the way a mouse does, which leaves the keyboard on it; the
// detail pane's own reveal is forgotten so only the chord's calls are counted.
const pickRow = async () => {
  withEntries([loginMeta({ id: 'l1' })])
  render(<Body />)
  await userEvent.click(screen.getByTestId('entry-item'))
  expect(document.activeElement).toBe(screen.getByTestId('entry-item'))
  clearCalls('copy_to_clipboard')
}

// fireEvent hands back whether the event survived, i.e. was not prevented.
const chord = (target: Element, code: string, init: KeyboardEventInit = { metaKey: true }) =>
  fireEvent.keyDown(target, { key: code.slice(3).toLowerCase(), code, ...init })

beforeEach(() => mockCommand('reveal_entry', () => entry))

afterEach(() => vi.restoreAllMocks())

describe('copy chords on a list row', () => {
  it.each([
    ['KeyB', 'ada@example.com'],
    ['KeyC', 'hunter2'],
    ['KeyT', '123456'],
    ['KeyY', 'hunter2123456'],
    ['KeyU', 'https://google.com']
  ])('%s copies its value and raises the pill', async (code, value) => {
    await pickRow()

    expect(chord(screen.getByTestId('entry-item'), code)).toBe(false)

    await vi.waitFor(() => expect(copied()).toEqual([value]))
    expect(useUi.getState().copied).toBe(true)
  })

  it('takes Ctrl as well as ⌘, and reads the physical key on any layout', async () => {
    await pickRow()

    // ⌃C on a Cyrillic layout: the key prints `с`, the code is still KeyC.
    fireEvent.keyDown(screen.getByTestId('entry-item'), { key: 'с', code: 'KeyC', ctrlKey: true })

    await vi.waitFor(() => expect(copied()).toEqual(['hunter2']))
  })

  it('cuts the code from the seed rather than copying the seed', async () => {
    await pickRow()

    chord(screen.getByTestId('entry-item'), 'KeyT')

    await vi.waitFor(() => expect(calls('generate_otp')).toContainEqual({ secret: 'JBSWY3DPEHPK3PXP' }))
    await vi.waitFor(() => expect(copied()).toEqual(['123456']))
  })

  it('copies nothing when the entry has no such value', async () => {
    mockCommand('reveal_entry', () => ({ ...entry, otp: '' }))
    await pickRow()
    clearCalls('reveal_entry')

    chord(screen.getByTestId('entry-item'), 'KeyT')

    await vi.waitFor(() => expect(calls('reveal_entry')).toHaveLength(1))
    expect(calls('generate_otp')).toHaveLength(0)
    expect(copied()).toEqual([])
  })
})

describe('the password and code together', () => {
  it.each([
    ['one-time code', { otp: '' }],
    ['password', { password: '' }]
  ])('copy nothing for a login with no %s', async (_missing, without) => {
    mockCommand('reveal_entry', () => ({ ...entry, ...without }))
    await pickRow()
    clearCalls('reveal_entry')

    chord(screen.getByTestId('entry-item'), 'KeyY')

    await vi.waitFor(() => expect(calls('reveal_entry')).toHaveLength(1))
    expect(copied()).toEqual([])
    expect(useUi.getState().copied).toBe(false)
  })
})

describe('copy chords from the search field', () => {
  it.each([
    ['KeyB', 'ada@example.com'],
    ['KeyT', '123456'],
    ['KeyU', 'https://google.com']
  ])('%s copies the selected row’s value, caret left in the field', async (code, value) => {
    await pickRow()
    const search = screen.getByTestId('search-input')
    search.focus()

    expect(chord(search, code)).toBe(false)

    await vi.waitFor(() => expect(copied()).toEqual([value]))
    expect(document.activeElement).toBe(search)
  })

  it('acts on the first row with nothing selected, like ⌘⏎', async () => {
    withEntries([loginMeta({ id: 'l1' })])
    render(<Body />)

    chord(screen.getByTestId('search-input'), 'KeyB')

    await vi.waitFor(() => expect(copied()).toEqual(['ada@example.com']))
  })

  // On a Mac, ⌃B and ⌃T in a text field move the caret back and transpose.
  it.each(['KeyB', 'KeyT'])('leaves ⌃%s to the Mac caret', async code => {
    await pickRow()
    clearCalls('reveal_entry')

    expect(chord(screen.getByTestId('search-input'), code, { ctrlKey: true })).toBe(true)

    expect(calls('reveal_entry')).toHaveLength(0)
    expect(copied()).toEqual([])
  })
})

describe('copy chords leave the system copy alone', () => {
  // ⌘C is the query's own copy there, and Ctrl+Y a PC's redo.
  it.each(['KeyC', 'KeyY'])('%s in the search field', async code => {
    await pickRow()
    clearCalls('reveal_entry')

    expect(chord(screen.getByTestId('search-input'), code)).toBe(true)

    expect(calls('reveal_entry')).toHaveLength(0)
    expect(copied()).toEqual([])
  })

  it('while the user has text selected', async () => {
    await pickRow()
    clearCalls('reveal_entry')
    vi.spyOn(window, 'getSelection').mockReturnValue({ toString: () => 'Google' } as Selection)

    expect(chord(screen.getByTestId('entry-item'), 'KeyC')).toBe(true)

    expect(calls('reveal_entry')).toHaveLength(0)
  })

  it('while a modal is open', async () => {
    await pickRow()
    clearCalls('reveal_entry')
    registerDialog('test')

    expect(chord(screen.getByTestId('entry-item'), 'KeyC')).toBe(true)
    unregisterDialog('test')

    expect(calls('reveal_entry')).toHaveLength(0)
  })

  it('with Shift or Alt held, which are other chords', async () => {
    await pickRow()
    clearCalls('reveal_entry')

    chord(screen.getByTestId('entry-item'), 'KeyC', { metaKey: true, shiftKey: true })
    chord(screen.getByTestId('entry-item'), 'KeyC', { metaKey: true, altKey: true })

    expect(calls('reveal_entry')).toHaveLength(0)
  })
})

describe('the palette', () => {
  it('lists the selected login’s copies with their chords, and runs them', async () => {
    withEntries([loginMeta({ id: 'l1' })])
    render(<Main />)
    await userEvent.click(screen.getByTestId('entry-item'))
    clearCalls('copy_to_clipboard')

    await userEvent.keyboard('{Meta>}k{/Meta}')
    await userEvent.keyboard('copy username')
    const palette = within(screen.getByTestId('command-palette'))
    expect(palette.getByText('Copy username')).toBeInTheDocument()
    expect(palette.getByText('⌘B')).toBeInTheDocument()

    await userEvent.keyboard('{Enter}')

    await vi.waitFor(() => expect(copied()).toEqual(['ada@example.com']))
  })

  it('lists the password and code together, under ⌘Y', async () => {
    withEntries([loginMeta({ id: 'l1' })])
    render(<Main />)
    await userEvent.click(screen.getByTestId('entry-item'))
    clearCalls('copy_to_clipboard')

    await userEvent.keyboard('{Meta>}k{/Meta}')
    await userEvent.keyboard('copy password and')
    const palette = within(screen.getByTestId('command-palette'))
    expect(palette.getByText('Copy password and one-time code')).toBeInTheDocument()
    expect(palette.getByText('⌘Y')).toBeInTheDocument()

    await userEvent.keyboard('{Enter}')

    await vi.waitFor(() => expect(copied()).toEqual(['hunter2123456']))
  })

  it('offers no copies with nothing selected', async () => {
    withEntries([loginMeta({ id: 'l1' })])
    render(<Main />)

    await userEvent.keyboard('{Meta>}k{/Meta}')

    const palette = within(screen.getByTestId('command-palette'))
    expect(palette.queryByText('Copy username')).not.toBeInTheDocument()
    expect(palette.queryByText('Copy password')).not.toBeInTheDocument()
  })
})
