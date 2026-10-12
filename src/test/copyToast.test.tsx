import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { act, render, screen } from '@testing-library/react'
import CopyToast from '@/components/elements/CopyToast'
import { flashCopied } from '@/store'
import { copy } from '@/services/copy'
import { calls } from './ipc'

beforeEach(() => vi.useFakeTimers())

afterEach(() => {
  // Drain whatever countdown the test left armed, so the module-level timer
  // never fires into the next test's store.
  act(() => vi.runOnlyPendingTimers())
  vi.useRealTimers()
})

describe('CopyToast', () => {
  it('renders nothing until something is copied', () => {
    const { container } = render(<CopyToast />)
    expect(container).toBeEmptyDOMElement()
  })

  // A control still on screen confirms in place, so a plain copy leaves the
  // pill down.
  it('stays down for a plain copy', () => {
    render(<CopyToast />)

    act(() => void copy('hunter2'))
    expect(calls('copy_to_clipboard')[0]).toMatchObject({ value: 'hunter2' })
    expect(screen.queryByTestId('copy-toast')).not.toBeInTheDocument()
  })

  it('appears when flashed and takes itself back down after two seconds', () => {
    render(<CopyToast />)

    act(() => flashCopied())
    expect(screen.getByTestId('copy-toast')).toHaveTextContent('Copied to Clipboard')

    act(() => void vi.advanceTimersByTime(1999))
    expect(screen.getByTestId('copy-toast')).toBeInTheDocument()

    act(() => void vi.advanceTimersByTime(1))
    expect(screen.queryByTestId('copy-toast')).not.toBeInTheDocument()
  })

  // The pill is up for two seconds from the *last* flash, so a quick second one
  // is not cut short by the first one's countdown.
  it('restarts the countdown on a second flash', () => {
    render(<CopyToast />)

    act(() => flashCopied())
    act(() => void vi.advanceTimersByTime(1500))
    act(() => flashCopied())

    act(() => void vi.advanceTimersByTime(1500))
    expect(screen.getByTestId('copy-toast')).toBeInTheDocument()

    act(() => void vi.advanceTimersByTime(500))
    expect(screen.queryByTestId('copy-toast')).not.toBeInTheDocument()
  })
})
