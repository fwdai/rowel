import { describe, it, expect, beforeEach, vi } from 'vitest'
import { fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import Aside from '@/components/Main/Body/Aside'
import Keyboard from '@/components/Main/Sidebar/Settings/Sections/Keyboard'
import { usePrefs, useUi } from '@/store'
import { withEntries, loginMeta, loginEntry } from './utils'
import { calls, clearCalls, mockCommand } from './ipc'

// Shortcuts moved to other keys in Settings › Keyboard shortcuts: the bindings
// follow, every hint names the new key, and the page records and refuses keys.

const entry = loginEntry({ id: 'l1', username: 'ada@example.com', password: 'hunter2' })
const copied = () => calls('copy_to_clipboard').map(args => args.value)
const remap = (shortcuts: Record<string, string>) => usePrefs.setState({ shortcuts })

beforeEach(() => mockCommand('reveal_entry', () => entry))

describe('a shortcut on another key', () => {
  it('runs from the new key and no longer from the old one', async () => {
    remap({ commands: 'KeyJ' })
    withEntries([loginMeta({ id: 'l1' })])
    render(<Main />)

    await userEvent.keyboard('{Meta>}k{/Meta}')
    expect(useUi.getState().palette).toBe(false)

    await userEvent.keyboard('{Meta>}j{/Meta}')
    expect(useUi.getState().palette).toBe(true)
  })

  it('copies from a row on the new key', async () => {
    remap({ copyUsername: 'KeyJ' })
    withEntries([loginMeta({ id: 'l1' })])
    render(<Main />)
    await userEvent.click(screen.getByTestId('entry-item'))
    clearCalls('copy_to_clipboard')
    const row = screen.getByTestId('entry-item')

    fireEvent.keyDown(row, { key: 'b', code: 'KeyB', metaKey: true })
    fireEvent.keyDown(row, { key: 'j', code: 'KeyJ', metaKey: true })

    await vi.waitFor(() => expect(copied()).toEqual(['ada@example.com']))
  })

  it('is named by its new key in the palette and the cheat sheet', async () => {
    remap({ copyUsername: 'KeyJ', lock: 'KeyO' })
    withEntries([loginMeta({ id: 'l1' })])
    const { unmount } = render(<Aside />)
    const sheet = within(screen.getByTestId('empty-select'))
    expect(sheet.getByText('⌘J')).toBeInTheDocument()
    expect(sheet.getByText('⌘O')).toBeInTheDocument()
    expect(sheet.queryByText('⌘B')).not.toBeInTheDocument()
    unmount()

    render(<Main />)
    await userEvent.keyboard('{Meta>}k{/Meta}')
    await userEvent.keyboard('lock')
    expect(within(screen.getByTestId('command-palette')).getByText('⌘O')).toBeInTheDocument()
  })
})

describe('Settings › Keyboard shortcuts', () => {
  const capture = (id: string) => screen.getByTestId(`settings-shortcut-${id}`)
  const record = async (id: string, key: string) => {
    await userEvent.click(capture(id))
    expect(capture(id)).toHaveTextContent('Press a key…')
    await userEvent.keyboard(key)
  }

  it('records a new key and stores only that change', async () => {
    render(<Keyboard />)
    expect(capture('lock')).toHaveTextContent('⌘L')

    await record('lock', '{Meta>}j{/Meta}')

    expect(capture('lock')).toHaveTextContent('⌘J')
    expect(calls('set_settings')).toContainEqual({ patch: { shortcuts: { lock: 'KeyJ' } } })
  })

  it('refuses a key another action has, a system key and a text-editing key', async () => {
    render(<Keyboard />)

    await record('lock', 'b')
    expect(screen.getByText('⌘B is already used by “Copy username”')).toBeInTheDocument()

    await record('lock', 'q')
    expect(screen.getByText('⌘Q belongs to the system')).toBeInTheDocument()

    await record('lock', 'v')
    expect(screen.getByText('⌘V is kept for editing text')).toBeInTheDocument()

    expect(capture('lock')).toHaveTextContent('⌘L')
    expect(usePrefs.getState().shortcuts).toEqual({})
  })

  it('gives up on Escape', async () => {
    render(<Keyboard />)

    await record('lock', '{Escape}')

    expect(capture('lock')).toHaveTextContent('⌘L')
    expect(calls('set_settings')).toHaveLength(0)
  })

  // WebKit on macOS leaves a clicked button unfocused; a bare click event is
  // the same here, since it moves no focus either.
  it('takes the keyboard when clicked, even where a click moves no focus', async () => {
    render(<Keyboard />)

    fireEvent.click(capture('lock'))
    expect(document.activeElement).toBe(capture('lock'))
    await userEvent.keyboard('j')

    expect(capture('lock')).toHaveTextContent('⌘J')
  })

  it('restores every default at once', async () => {
    remap({ lock: 'KeyJ', copyUrl: 'KeyO' })
    render(<Keyboard />)

    await userEvent.click(screen.getByTestId('settings-shortcuts-reset'))

    expect(usePrefs.getState().shortcuts).toEqual({})
    expect(capture('copyUrl')).toHaveTextContent('⌘U')
    expect(screen.getByTestId('settings-shortcuts-reset')).toBeDisabled()
  })
})
