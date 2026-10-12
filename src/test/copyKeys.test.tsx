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
 * ⌘T one-time code, ⌘U URL — on a focused row only, so ⌘C everywhere else is
 * still the system copy of whatever text is selected.
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

describe('copy chords leave the system copy alone', () => {
  it('in the search field', async () => {
    await pickRow()
    clearCalls('reveal_entry')

    expect(chord(screen.getByTestId('search-input'), 'KeyC')).toBe(true)

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

  it('offers no copies with nothing selected', async () => {
    withEntries([loginMeta({ id: 'l1' })])
    render(<Main />)

    await userEvent.keyboard('{Meta>}k{/Meta}')

    const palette = within(screen.getByTestId('command-palette'))
    expect(palette.queryByText('Copy username')).not.toBeInTheDocument()
    expect(palette.queryByText('Copy password')).not.toBeInTheDocument()
  })
})
