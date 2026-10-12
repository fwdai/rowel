import { describe, it, expect } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import { useLayout } from '@/hooks/useLayout'
import { useVisualViewport, viewportStyle } from '@/hooks/useVisualViewport'
import { setLayout } from './layout'

describe('useLayout', () => {
  it('reads wide when the compact query does not match', () => {
    expect(renderHook(() => useLayout()).result.current).toBe('wide')
  })

  it('reads compact when the query matches', () => {
    setLayout('compact')
    expect(renderHook(() => useLayout()).result.current).toBe('compact')
  })

  it('falls back to wide where matchMedia is missing', () => {
    const original = window.matchMedia
    // @ts-expect-error — modelling a webview without the API at all.
    delete window.matchMedia
    expect(renderHook(() => useLayout()).result.current).toBe('wide')
    window.matchMedia = original
  })
})

describe('useVisualViewport', () => {
  it('is null where the API is absent', () => {
    expect(renderHook(() => useVisualViewport()).result.current).toBeNull()
  })

  const install = (height: number, offsetTop: number) =>
    Object.defineProperty(window, 'visualViewport', {
      configurable: true,
      value: { height, offsetTop, addEventListener: () => {}, removeEventListener: () => {} }
    })
  const uninstall = () =>
    Object.defineProperty(window, 'visualViewport', { configurable: true, value: undefined })

  // The keyboard is only ever up for a field, so the viewport is only ever
  // read for one: with nothing to type into the shell keeps CSS's full height,
  // whatever a stale reading says (the Simulator's accessory strip, left
  // behind after a field went away).
  it('is null while nothing takes typing, whatever the API reports', () => {
    install(640, 120)
    expect(renderHook(() => useVisualViewport()).result.current).toBeNull()

    const button = document.createElement('button')
    document.body.append(button)
    button.focus()
    expect(renderHook(() => useVisualViewport()).result.current).toBeNull()
    button.remove()
    uninstall()
  })

  it('reports the visible height and how far iOS panned it while a field has focus', () => {
    install(640, 120)
    const input = document.createElement('input')
    document.body.append(input)
    input.focus()
    const { result } = renderHook(() => useVisualViewport())
    expect(result.current).toEqual({ height: 640, offsetTop: 120 })

    // Leaving the field ends the session on the spot, before the viewport
    // has said anything about the keyboard going.
    act(() => input.blur())
    expect(result.current).toBeNull()

    // So does the field going away under the caret: an unmount fires no blur,
    // but the next focus change reads the document as it now is.
    input.focus()
    expect(renderHook(() => useVisualViewport()).result.current).not.toBeNull()
    input.remove()
    act(() => document.body.dispatchEvent(new FocusEvent('focusout', { bubbles: true })))
    expect(renderHook(() => useVisualViewport()).result.current).toBeNull()
    uninstall()
  })

  it('turns that into a height plus a translate, and no translate when unpanned', () => {
    expect(viewportStyle(null)).toBeUndefined()
    expect(viewportStyle({ height: 640, offsetTop: 0 })).toEqual({
      height: 640,
      transform: undefined
    })
    expect(viewportStyle({ height: 640, offsetTop: 120 })).toEqual({
      height: 640,
      transform: 'translateY(120px)'
    })
  })
})
