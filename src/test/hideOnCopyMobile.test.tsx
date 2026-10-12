import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import SessionGroup from '@/components/Main/Sidebar/Settings/Sections/Security/SessionGroup'
import { copy } from '@/services/copy'
import { usePrefs } from '@/store'
import { calls } from './ipc'

// The phone build, where `hide_window` is not registered at all: a call would
// be an IPC error, so both guards — the toggle and the copy — must hold even
// with the setting carried over on as it might be from a desktop's file.
vi.mock('@/lib/platform', () => ({
  isIOS: true,
  isAndroid: false,
  isMobile: true,
  isPC: false,
  chord: (key: string) => `⌘${key}`
}))

describe('hide on copy, on a phone', () => {
  it('never asks to hide, even with the setting on', async () => {
    usePrefs.setState({ hideOnCopy: true })
    await copy('hunter2')
    expect(calls('copy_to_clipboard')).toHaveLength(1)
    expect(calls('hide_window')).toHaveLength(0)
  })

  it('has no switch in Settings', () => {
    render(<SessionGroup />)
    expect(screen.queryByTestId('settings-hide-on-copy-toggle')).not.toBeInTheDocument()
  })
})
