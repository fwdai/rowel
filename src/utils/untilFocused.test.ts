import { beforeEach, describe, expect, it, vi } from 'vitest'
import { untilFocused } from './untilFocused'

// What the window says each time it is asked, in order; the last answer
// repeats. An Error in the list is a query that fails.
let answers: Array<boolean | Error> = []
let asked = 0

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    isFocused: () => {
      asked++
      const answer = answers.length > 1 ? answers.shift() : answers[0]
      return answer instanceof Error ? Promise.reject(answer) : Promise.resolve(answer ?? true)
    }
  })
}))

describe('untilFocused', () => {
  beforeEach(() => {
    answers = []
    asked = 0
  })

  it('returns at once when the window is already in front', async () => {
    answers = [true]
    await expect(untilFocused(1000, 0)).resolves.toBe(true)
    expect(asked).toBe(1)
  })

  // The Face ID case: the sheet is still coming down on the first asks.
  it('keeps asking until the window says it is in front', async () => {
    answers = [false, false, true]
    await expect(untilFocused(1000, 0)).resolves.toBe(true)
    expect(asked).toBe(3)
  })

  it('gives up, saying so, when the window never comes to the front in time', async () => {
    answers = [false]
    await expect(untilFocused(30, 0)).resolves.toBe(false)
    expect(asked).toBeGreaterThan(1)
  })

  // A window that cannot be asked has nothing to wait for.
  it('treats a failed query as in front', async () => {
    answers = [new Error('no window')]
    await expect(untilFocused(1000, 0)).resolves.toBe(true)
    expect(asked).toBe(1)
  })

  // tao's Android window says `false` whatever the truth: asking it would hold
  // every unlock for the whole limit.
  it('does not ask on Android, where the answer means nothing', async () => {
    vi.resetModules()
    vi.doMock('@/lib/platform', () => ({ isAndroid: true }))
    try {
      const { untilFocused: onAndroid } = await import('./untilFocused')
      answers = [false]
      await expect(onAndroid(1000, 0)).resolves.toBe(true)
      expect(asked).toBe(0)
    } finally {
      vi.doUnmock('@/lib/platform')
      vi.resetModules()
    }
  })
})
