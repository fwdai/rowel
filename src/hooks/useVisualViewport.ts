import { useSyncExternalStore, type CSSProperties } from 'react'

export interface VisualViewport {
  /** The height still visible above the on-screen keyboard. */
  height: number
  /** How far iOS has panned the visible area down from the layout viewport's top. */
  offsetTop: number
}

const viewport = () => (typeof window === 'undefined' ? null : (window.visualViewport ?? null))

// Whether the keyboard has a reason to be up: an element that takes typing
// holds the focus. A button or a link never raises it. Read off the document
// rather than remembered from the last focus event, so the answer is the
// current one however the focus moved — including a focused field being
// unmounted under the keyboard, which fires no blur at all: React reads the
// snapshot again on every render of the caller, and the shell renders as the
// editor leaves, so that read is what takes it back to full height.
const typing = (): boolean => {
  if (typeof document === 'undefined') return false
  const active = document.activeElement
  if (!(active instanceof HTMLElement)) return false
  if (active instanceof HTMLTextAreaElement || active.isContentEditable) return true
  if (!(active instanceof HTMLInputElement)) return false
  return !['button', 'checkbox', 'radio', 'submit', 'reset', 'file', 'range'].includes(
    active.type
  )
}

const subscribe = (onChange: () => void) => {
  const view = viewport()
  view?.addEventListener('resize', onChange)
  view?.addEventListener('scroll', onChange)
  // Focus moving is what starts and ends a keyboard session, so it is a change
  // of the snapshot in its own right — the viewport's own events come later
  // (the keyboard's animation), or on the Simulator sometimes not at all.
  document.addEventListener('focusin', onChange)
  document.addEventListener('focusout', onChange)
  return () => {
    view?.removeEventListener('resize', onChange)
    view?.removeEventListener('scroll', onChange)
    document.removeEventListener('focusin', onChange)
    document.removeEventListener('focusout', onChange)
  }
}

// `useSyncExternalStore` re-renders on every snapshot that is not `===` the
// last, so the object is reused until one of its two numbers actually moves.
let last: VisualViewport | null = null
const snapshot = (): VisualViewport | null => {
  const view = viewport()
  if (!view || !typing()) return (last = null)
  if (last?.height !== view.height || last.offsetTop !== view.offsetTop) {
    last = { height: view.height, offsetTop: view.offsetTop }
  }
  return last
}

/**
 * The part of the page the user can actually see once the keyboard is up —
 * and nothing at all while it is not.
 *
 * iOS does not shrink the layout viewport when the keyboard comes up: a
 * `height: 100%` shell keeps its full height and pushes the focused input under
 * the keys, and when the caret then has to be scrolled into view iOS pans the
 * visible area down instead, leaving anything anchored to the layout top out
 * of sight. Sizing the compact shell and its sheets from this — height and
 * offset both, see [`viewportStyle`] — keeps the caret in view.
 *
 * Only while something is being typed into, though. At rest the visual
 * viewport ought to be the window, but it is not always reported so: the
 * Simulator with a hardware keyboard attached raises only the accessory strip
 * for a field, and does not always say when it has gone, so a shell that
 * trusted the number at rest kept the strip's height off its bottom for good
 * — a band of the page's ground under the tab bar. With no field focused the
 * answer is null and the shell is the plain full-height box CSS makes it, so
 * no stale reading can ever shorten it. Null too where the API is absent
 * (desktop webviews, jsdom), and callers then leave it to CSS.
 */
export const useVisualViewport = (): VisualViewport | null =>
  useSyncExternalStore(subscribe, snapshot, () => null)

// The inline style that pins an element to the visible area: its height, and a
// translate for however far iOS has panned it. Both callers (the compact shell,
// the sheet frame) need the same two lines, so they live here.
export const viewportStyle = (view: VisualViewport | null): CSSProperties | undefined =>
  view
    ? {
        height: view.height,
        transform: view.offsetTop ? `translateY(${view.offsetTop}px)` : undefined
      }
    : undefined
