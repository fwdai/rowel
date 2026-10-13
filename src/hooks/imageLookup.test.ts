import { describe, it, expect } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import { imageLookup } from './imageLookup'

// A fetch whose answers the test hands out one call at a time.
const controlled = () => {
  const calls: { key: string; answer: (uri: string | null) => void }[] = []
  const fetchImage = (key: string) =>
    new Promise<string | null>(answer => calls.push({ key, answer }))
  return { calls, fetchImage }
}

describe('imageLookup', () => {
  it('asks once per key and fans the answer out', async () => {
    const { calls, fetchImage } = controlled()
    const { useImage } = imageLookup(fetchImage)
    const first = renderHook(() => useImage('UKR'))
    const second = renderHook(() => useImage('UKR'))

    expect(calls).toHaveLength(1)
    await act(async () => calls[0].answer('data:ukr'))
    expect(first.result.current).toBe('data:ukr')
    expect(second.result.current).toBe('data:ukr')
  })

  // The vault locks (or the workspace switches) while a lookup is in flight:
  // its late answer is the old session's, and must not reach the new one.
  it('drops an answer that arrives after a reset', async () => {
    const { calls, fetchImage } = controlled()
    const { useImage, reset } = imageLookup(fetchImage)
    const before = renderHook(() => useImage('UKR'))

    before.unmount()
    reset()
    const after = renderHook(() => useImage('UKR'))
    expect(calls).toHaveLength(2)

    await act(async () => calls[0].answer('data:old-vault'))
    expect(after.result.current).toBeNull()
    // Nor did it clear the new lookup's pending mark: no third request.
    renderHook(() => useImage('UKR'))
    expect(calls).toHaveLength(2)

    await act(async () => calls[1].answer('data:new-vault'))
    expect(after.result.current).toBe('data:new-vault')
  })
})
