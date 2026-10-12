import { describe, it, expect, beforeEach } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import CopyButton from '@/components/elements/CopyButton'
import Generator from '@/components/Main/Generator'
import SessionGroup from '@/components/Main/Sidebar/Settings/Sections/Security/SessionGroup'
import { copy } from '@/services/copy'
import { openGenerator, usePrefs } from '@/store'
import { calls, mockCommand, mockCommandOnce } from './ipc'

const turnOn = () => usePrefs.setState({ hideOnCopy: true })

beforeEach(() => mockCommand('generate_password', () => 'Generated123!'))

// "Hide Rowel after copying": a copy hands focus back to the app the user came
// from, but only when asked to, and never before the value is on the clipboard.
describe('hide on copy', () => {
  it('leaves the window alone while the setting is off', async () => {
    await copy('hunter2')
    expect(calls('copy_to_clipboard')).toHaveLength(1)
    expect(calls('hide_window')).toHaveLength(0)
  })

  it('hides once the clipboard write has landed', async () => {
    turnOn()
    let land!: () => void
    mockCommandOnce('copy_to_clipboard', () => new Promise<void>(resolve => (land = resolve)))

    const copying = copy('hunter2')
    await Promise.resolve()
    expect(calls('hide_window')).toHaveLength(0)

    land()
    await copying
    expect(calls('hide_window')).toHaveLength(1)
  })

  it('stays when the clipboard write fails', async () => {
    turnOn()
    mockCommandOnce('copy_to_clipboard', () => Promise.reject({ kind: 'other', message: 'no' }))
    await expect(copy('hunter2')).rejects.toBeTruthy()
    expect(calls('hide_window')).toHaveLength(0)
  })

  it('stays for a copy that opts out', async () => {
    turnOn()
    await copy('hunter2', { stay: true })
    expect(calls('copy_to_clipboard')).toHaveLength(1)
    expect(calls('hide_window')).toHaveLength(0)
  })

  it('hides from a copy button, unless the button says to stay', async () => {
    turnOn()
    const { rerender } = render(<CopyButton value="hunter2" testid="copy" />)
    await userEvent.click(screen.getByTestId('copy'))
    await waitFor(() => expect(calls('hide_window')).toHaveLength(1))

    rerender(<CopyButton value="hunter2" testid="copy" stay />)
    await userEvent.click(screen.getByTestId('copy'))
    await waitFor(() => expect(calls('copy_to_clipboard')).toHaveLength(2))
    expect(calls('hide_window')).toHaveLength(1)
  })

  // The generator is work still going on inside Rowel.
  it('stays for the generator’s use & copy', async () => {
    turnOn()
    openGenerator()
    render(<Generator />)
    expect(await screen.findByText('Generated123!')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('generator-use-button'))
    await waitFor(() => expect(calls('copy_to_clipboard')).toHaveLength(1))
    expect(calls('hide_window')).toHaveLength(0)
  })

  it('is a switch in Settings, off by default', async () => {
    render(<SessionGroup />)
    const toggle = screen.getByTestId('settings-hide-on-copy-toggle')
    expect(toggle).toHaveAttribute('aria-checked', 'false')

    await userEvent.click(toggle)
    expect(toggle).toHaveAttribute('aria-checked', 'true')
    expect(calls('set_settings')).toContainEqual({ patch: { hideOnCopy: true } })
  })
})
