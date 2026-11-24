import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import { useBackButton } from '@/hooks/useBackButton'
import {
  lockSettings,
  openAddPicker,
  openGenerator,
  openSettings,
  startEntry,
  useUi,
  useVault
} from '@/store'
import { withEntries, loginEntry, loginMeta } from './utils'
import { setLayout } from './layout'
import { mockCommand } from './ipc'

// The Android build: Back is a press the app hears through Tauri's listener.
vi.mock('@/lib/platform', () => ({
  isIOS: false,
  isAndroid: true,
  isMobile: true,
  isPC: false,
  chord: (key: string) => `⌘${key}`
}))

const back = vi.hoisted(() => ({
  press: (() => {}) as (payload: { canGoBack: boolean }) => void,
  unregister: vi.fn()
}))

vi.mock('@tauri-apps/api/app', () => ({
  onBackButtonPress: vi.fn(async (handler: (payload: { canGoBack: boolean }) => void) => {
    back.press = handler
    return { unregister: back.unregister.mockResolvedValue(undefined) }
  })
}))

const press = () => act(() => back.press({ canGoBack: false }))

// The activity's side of the shell (`ShellBridge.kt`): only `leave` is asked
// for here.
const leave = vi.fn()

// What `App` mounts it with: the whole phone shell under the one listener.
function Phone() {
  useBackButton()
  return <Main />
}

const titleInput = () => document.querySelector<HTMLInputElement>('input[name="title"]')!

beforeEach(() => {
  vi.clearAllMocks()
  setLayout('compact')
  mockCommand('generate_password', () => 'Generated123!')
  window.__ROWEL_ANDROID__ = { insets: () => '{}', setDark: vi.fn(), leave }
  withEntries([loginMeta({ id: 'l1', title: 'Google' }), loginMeta({ id: 'l2', title: 'Airbnb' })])
})

afterEach(() => {
  delete window.__ROWEL_ANDROID__
})

describe('Android’s Back', () => {
  it('pops the detail to the list, and leaves the app from the list', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'l1', title: 'Google' }))
    render(<Phone />)
    await userEvent.click(screen.getByText('Google'))
    expect(useVault.getState().currentId).toBe('l1')

    press()
    expect(useVault.getState().currentId).toBeNull()
    expect(leave).not.toHaveBeenCalled()

    press()
    expect(leave).toHaveBeenCalledOnce()
  })

  it('closes the add picker, stepping back out of its question first', async () => {
    render(<Phone />)
    act(() => openAddPicker())
    await userEvent.click(screen.getByTestId('add-kind-identity'))
    expect(screen.queryByTestId('add-kind-login')).not.toBeInTheDocument()

    press()
    expect(screen.getByTestId('add-kind-login')).toBeInTheDocument()
    expect(useUi.getState().addPicker).toBe(true)

    press()
    expect(useUi.getState().addPicker).toBe(false)
    expect(leave).not.toHaveBeenCalled()
  })

  // The form's own Cancel, guard and all: a draft with typing in it is not
  // dropped by one stray press.
  it('cancels a draft the way its Cancel does, armed by the first press', async () => {
    render(<Phone />)
    act(() => startEntry('login'))
    await userEvent.type(titleInput(), 'Netflix')

    press()
    expect(screen.getByTestId('cancel-entry-button')).toHaveTextContent('Discard changes?')
    expect(useVault.getState().creating).toBe('login')

    press()
    expect(useVault.getState().creating).toBeNull()
    expect(leave).not.toHaveBeenCalled()
  })

  it('closes a sheet over the form before it touches the form', () => {
    render(<Phone />)
    act(() => startEntry('login'))
    act(() => openGenerator(() => {}))
    expect(screen.getByTestId('generator-dialog')).toBeInTheDocument()

    press()
    expect(screen.queryByTestId('generator-dialog')).not.toBeInTheDocument()
    expect(useVault.getState().creating).toBe('login')
  })

  it('steps out of a settings sub-page, then the section, then the app', async () => {
    render(<Phone />)
    act(() => openSettings())
    await userEvent.click(screen.getByTestId('settings-nav-workspaces'))
    await userEvent.click(screen.getByTestId('workspace-add-row'))
    await userEvent.click(screen.getByTestId('workspace-new-row'))
    expect(screen.getByTestId('settings-subpage')).toBeInTheDocument()

    press()
    expect(screen.queryByTestId('settings-subpage')).not.toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Lockboxes' })).toBeInTheDocument()

    press()
    expect(screen.queryByTestId('settings-pane')).not.toBeInTheDocument()
    expect(screen.getByTestId('settings-nav-workspaces')).toBeInTheDocument()
    expect(leave).not.toHaveBeenCalled()

    press()
    expect(leave).toHaveBeenCalledOnce()
  })

  // The section's Back is shown inert under the lock, and the press is refused
  // the same way — not taken as leave to quit the app either.
  it('is held by a locked settings section', async () => {
    render(<Phone />)
    act(() => openSettings())
    await userEvent.click(screen.getByTestId('settings-nav-workspaces'))
    act(() => lockSettings(true))

    press()
    expect(screen.getByRole('heading', { name: 'Lockboxes' })).toBeInTheDocument()
    expect(leave).not.toHaveBeenCalled()

    act(() => lockSettings(false))
    press()
    expect(screen.queryByTestId('settings-pane')).not.toBeInTheDocument()
  })

  // Even when it goes before the listener's registration has come back.
  it('stops listening when the app unmounts', async () => {
    const { unmount } = render(<Phone />)
    unmount()
    await waitFor(() => expect(back.unregister).toHaveBeenCalledOnce())
  })
})
