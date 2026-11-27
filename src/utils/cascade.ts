// The entrance of one piece: it comes up into place out of a soft focus. The
// curve spends its speed at once and takes its time settling, so the piece is
// readable almost as soon as it starts and still seen to land.
const FROM = { opacity: 0, transform: 'translateY(12px) scale(0.985)', filter: 'blur(6px)' }
const TO = { opacity: 1, transform: 'none', filter: 'blur(0px)' }
const SETTLE = 'cubic-bezier(0.16, 1, 0.3, 1)'
const DURATION = 420
// Each piece starts this long after the one before it, up to a limit: a long
// view must not keep its last rows waiting.
const STEP = 28
const LONGEST_WAIT = 280
// How far a view is taken apart: its sections, a section's groups, a group's rows.
const DEPTH = 4

// What a view is made of, as far as the eye is concerned: things that take up
// room in the flow. An overlay pinned over its parent (a sheen, a badge) is
// part of that parent, not a thing of its own.
function parts(el: HTMLElement): HTMLElement[] {
  return [...el.children].filter((child): child is HTMLElement => {
    if (!(child instanceof HTMLElement)) return false
    const { width, height } = child.getBoundingClientRect()
    return width > 0 && height > 0 && getComputedStyle(child).position !== 'absolute'
  })
}

// A surface of its own — a card, a face — rather than a wrapper that only
// arranges what is in it.
function paints(el: HTMLElement): boolean {
  const style = getComputedStyle(el)
  return style.borderTopWidth !== '0px' || style.backgroundColor !== 'rgba(0, 0, 0, 0)'
}

interface Found {
  /** What comes in one by one, top to bottom. */
  pieces: HTMLElement[]
  /** The surfaces those were taken out of, with the piece each one opens on. */
  frames: [frame: HTMLElement, first: number][]
}

// A stack — things one under another, each as wide as the stack — is taken
// apart into its layers, and they into theirs. Anything else (a title beside
// its icon, two columns, a single block) comes in whole.
function find(el: HTMLElement, depth: number, found: Found): void {
  const inside = parts(el)
  const width = el.getBoundingClientRect().width
  const stack =
    inside.length > 1 && inside.every(part => part.getBoundingClientRect().width >= width * 0.8)
  if (depth === 0 || !stack) {
    found.pieces.push(el)
    return
  }
  if (paints(el)) found.frames.push([el, found.pieces.length])
  for (const part of inside) find(part, depth - 1, found)
}

/**
 * The entrance of a view that has just been drawn: everything under a
 * `data-cascade` element in `scope` comes in piece by piece, top to bottom —
 * the title, then each row of each card, then what stands under them — each
 * rising into place out of a soft focus a beat after the one above it.
 *
 * The pieces are found by shape (see `find`), not named, so a view needs
 * nothing but the attribute, and a kind of entry laid out some other way
 * still arrives as itself.
 *
 * `quick` is for a view that follows hard on another — a list stepped through
 * with a held key: no choreography, only the edge taken off the cut, so each
 * one can be read as it passes.
 *
 * Nothing is left behind on the elements when it is over (no fill), so none
 * of them stays a stacking context that would hold an overlay of its own
 * under the rest of the window. Skipped where the user asked for less motion.
 */
export function cascade(scope: ParentNode | null, quick = false): void {
  if (!scope || !('animate' in Element.prototype)) return
  if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return

  for (const root of scope.querySelectorAll<HTMLElement>('[data-cascade]')) {
    if (quick) {
      root.animate([{ opacity: 0.4 }, { opacity: 1 }], { duration: 120, easing: 'ease-out' })
      continue
    }
    const found: Found = { pieces: [], frames: [] }
    find(root, DEPTH, found)
    const wait = (index: number) => Math.min(index * STEP, LONGEST_WAIT)
    found.pieces.forEach((piece, index) =>
      piece.animate([FROM, TO], {
        duration: DURATION,
        delay: wait(index),
        easing: SETTLE,
        fill: 'backwards'
      })
    )
    // A surface fades up as its first piece sets off, so its pieces never
    // arrive on a card that was standing there empty.
    for (const [frame, first] of found.frames)
      frame.animate([{ opacity: 0 }, { opacity: 1 }], {
        duration: DURATION / 2,
        delay: wait(first),
        easing: 'ease-out',
        fill: 'backwards'
      })
  }
}
