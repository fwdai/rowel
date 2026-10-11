import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { act, fireEvent, render, screen } from '@testing-library/react'
import CopyValue from '@/components/elements/fields/CopyValue'
import { useUi } from '@/store'
import { calls } from './ipc'

beforeEach(() => vi.useFakeTimers())

afterEach(() => {
  act(() => vi.runOnlyPendingTimers())
  vi.useRealTimers()
})

describe('CopyValue', () => {
  it('copies the real value and confirms in place, not with the pill', () => {
    render(
      <CopyValue value="hunter2" label="Password · Copy" testid="value">
        ••••
      </CopyValue>
    )

    fireEvent.click(screen.getByTestId('value'))

    expect(calls('copy_to_clipboard')[0]).toMatchObject({ value: 'hunter2' })
    expect(screen.getByTestId('copied-mark')).toBeInTheDocument()
    // The check is a glyph, so the value is still the button's whole text.
    expect(screen.getByTestId('value')).toHaveTextContent(/^••••$/)
    expect(useUi.getState().copied).toBe(false)

    act(() => void vi.advanceTimersByTime(1200))
    expect(screen.queryByTestId('copied-mark')).not.toBeInTheDocument()
  })
})
