import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { followSystemInsets, INSETS_EVENT, leaveApp, tintSystemBars } from './android'

const PROPERTIES = ['--safe-top', '--safe-right', '--safe-bottom', '--safe-left', '--bar-bottom']
const root = () => document.documentElement.style

// The activity's side, as `ShellBridge.kt` puts it on the window.
const install = (insets: string) => {
  const bridge = { insets: vi.fn(() => insets), setDark: vi.fn(), leave: vi.fn() }
  window.__ROWEL_ANDROID__ = bridge
  return bridge
}

beforeEach(() => {
  // A 3x phone: the activity measures in device pixels.
  Object.defineProperty(window, 'devicePixelRatio', { value: 3, configurable: true })
})

afterEach(() => {
  delete window.__ROWEL_ANDROID__
  for (const property of PROPERTIES) root().removeProperty(property)
})

describe('the system insets', () => {
  it('are written over the stylesheet’s, in CSS pixels', () => {
    install('{"top":72,"right":0,"bottom":144,"left":0,"bar":144}')
    followSystemInsets()

    expect(root().getPropertyValue('--safe-top')).toBe('24px')
    expect(root().getPropertyValue('--safe-right')).toBe('0px')
    expect(root().getPropertyValue('--safe-bottom')).toBe('48px')
    expect(root().getPropertyValue('--safe-left')).toBe('0px')
    expect(root().getPropertyValue('--bar-bottom')).toBe('48px')
  })

  // A rotation, a switch to gesture navigation, the keyboard coming up: the
  // activity says the answer changed, and the page reads it again.
  it('follow the activity when the edges move', () => {
    const bridge = install('{"top":72,"right":0,"bottom":144,"left":0,"bar":144}')
    followSystemInsets()

    bridge.insets.mockReturnValue('{"top":72,"right":0,"bottom":0,"left":0,"bar":0}')
    window.dispatchEvent(new Event(INSETS_EVENT))

    expect(root().getPropertyValue('--safe-bottom')).toBe('0px')
    expect(root().getPropertyValue('--bar-bottom')).toBe('0px')
  })

  // Nothing measured yet, or an answer that is not numbers: the stylesheet's
  // `env()` defaults stand rather than a zero nobody measured.
  it('leave an edge they were not told about at the stylesheet’s default', () => {
    install('{"top":"72","bottom":null}')
    followSystemInsets()
    install('not json')
    window.dispatchEvent(new Event(INSETS_EVENT))
    install('{}')
    window.dispatchEvent(new Event(INSETS_EVENT))

    for (const property of PROPERTIES) expect(root().getPropertyValue(property)).toBe('')
  })

  it('are left to the stylesheet without the activity', () => {
    followSystemInsets()
    expect(root().getPropertyValue('--safe-top')).toBe('')
  })
})

describe('the activity’s controls', () => {
  it('tint the bars’ icons for the app’s theme, not the phone’s', () => {
    const bridge = install('{}')
    tintSystemBars('dark')
    expect(bridge.setDark).toHaveBeenLastCalledWith(true)
    tintSystemBars('light')
    expect(bridge.setDark).toHaveBeenLastCalledWith(false)
  })

  it('leave the app when asked', () => {
    const bridge = install('{}')
    leaveApp()
    expect(bridge.leave).toHaveBeenCalledOnce()
  })
})
