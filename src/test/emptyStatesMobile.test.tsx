import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { SelectEmpty, VaultEmpty } from '@/components/Main/Body/Empty/variants'

// A phone or tablet build: no keyboard to hold a chord with, so the empty
// states name no shortcuts.
vi.mock('@/lib/platform', () => ({
  isIOS: true,
  isAndroid: false,
  isMobile: true,
  isPC: false,
  chord: (key: string) => `⌘${key}`
}))

describe('empty states, on a phone', () => {
  it('shows no shortcut sheet when nothing is selected', () => {
    render(<SelectEmpty />)

    expect(screen.getByText('Select an item')).toBeInTheDocument()
    expect(screen.queryByText('↑↓')).not.toBeInTheDocument()
    expect(screen.queryByText('⌘K')).not.toBeInTheDocument()
  })

  it('keeps the empty vault’s actions but drops its shortcut hints', () => {
    render(<VaultEmpty />)

    expect(screen.getByTestId('create-first-entry-button')).toBeInTheDocument()
    expect(screen.queryByText('⌘N')).not.toBeInTheDocument()
    expect(screen.queryByText('⌘K')).not.toBeInTheDocument()
  })
})
