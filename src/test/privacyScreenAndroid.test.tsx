import { describe, it, expect, vi } from 'vitest'
import { act, render, screen } from '@testing-library/react'
import Main from '@/components/Main'
import { withEntries, loginMeta } from './utils'

vi.mock('@/lib/platform', () => ({
  isIOS: false,
  isAndroid: true,
  isMobile: true,
  isPC: false,
  chord: (key: string) => `⌘${key}`
}))

// What tao's Android window answers: never focused, and no focus event unless
// the activity actually loses and regains it — which an in-app unlock is not.
const { isFocused } = vi.hoisted(() => ({ isFocused: vi.fn().mockResolvedValue(false) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onFocusChanged: vi.fn().mockResolvedValue(() => {}),
    isFocused
  })
}))

describe('the privacy cover, on Android', () => {
  it('is not raised over a vault just opened', async () => {
    withEntries([loginMeta({ id: 'l1', title: 'Google' })])
    render(<Main />)
    // Past the window's one-shot answer and a few of the re-checks a cover
    // raised on it would make.
    await act(() => new Promise(resolve => setTimeout(resolve, 700)))

    expect(screen.queryByTestId('privacy-screen')).not.toBeInTheDocument()
    expect(isFocused).not.toHaveBeenCalled()
  })
})
