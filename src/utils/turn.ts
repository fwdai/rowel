import { scrollParent } from './scrollParent'

/**
 * Which way a surface turned: 1 to something after what it showed (the next
 * row down a list), -1 to something before it, 0 to something that is neither
 * — the same thing in another mode, or the first thing it shows.
 */
export type Way = -1 | 0 | 1

/** What a surface showed, kept to be seen leaving after the surface has moved on. */
export type Leaving = HTMLElement

// How far each sheet travels. Short: the fade does the rest, and a sheet that
// crossed the whole pane would take the eye with it.
const TRAVEL = 24
// The whole turn, and the part of it the old sheet is still to be seen in. The
// two are never both plain to see: the old one is nearly gone before the new
// one shows, so the pane reads as one sheet moving off and another moving on,
// not as two laid over each other.
const TURN = 320
const HANDOVER = 90
// Away slowly and then fast; in fast and then a long time settling.
const AWAY = 'cubic-bezier(0.3, 0, 0.8, 0.15)'
const HOME = 'cubic-bezier(0.05, 0.7, 0.1, 1)'

const stageOf = (scope: ParentNode | null) =>
  scope?.querySelector<HTMLElement>('[data-stage]') ?? null

const still = () =>
  !('animate' in Element.prototype) ||
  window.matchMedia('(prefers-reduced-motion: reduce)').matches

/**
 * A picture of what `scope` shows now — the `data-stage` element in it — laid
 * exactly over the original, to be handed to `turn` once the surface has drawn
 * what comes next. A copy of the DOM rather than of the pixels, so it costs a
 * clone and nothing else; it answers to nothing (no pointer, no focus, no test
 * id) and is gone the moment it has left.
 */
export function capture(scope: ParentNode | null): Leaving | null {
  const stage = stageOf(scope)
  const pane = stage && scrollParent(stage)
  if (!stage || !pane || still()) return null

  const at = stage.getBoundingClientRect()
  const within = pane.getBoundingClientRect()
  const copy = stage.cloneNode(true) as HTMLElement
  for (const el of [copy, ...copy.querySelectorAll('[data-testid], [id], [data-stage]')]) {
    el.removeAttribute('data-testid')
    el.removeAttribute('id')
    el.removeAttribute('data-stage')
  }

  // The window the copy is seen through: the pane's own box, so a sheet
  // scrolled under the pane's edge is cut where the original was.
  const frame = document.createElement('div')
  frame.setAttribute('aria-hidden', 'true')
  frame.inert = true
  Object.assign(frame.style, {
    position: 'fixed',
    left: `${within.left}px`,
    top: `${within.top}px`,
    width: `${within.width}px`,
    height: `${within.height}px`,
    overflow: 'hidden',
    pointerEvents: 'none'
  })
  // The sheet stood in a container of its own width, and folds by it.
  const sheet = document.createElement('div')
  Object.assign(sheet.style, {
    position: 'absolute',
    left: `${at.left - within.left}px`,
    top: `${at.top - within.top}px`,
    width: `${at.width}px`,
    containerType: 'inline-size'
  })
  sheet.append(copy)
  frame.append(sheet)
  // In the pane, so the copy inherits what the original did (ink, type).
  pane.append(frame)
  // A margin that ran out through the original's top edge is held inside the
  // copy's own box: put the copy back by however far that moved it.
  const now = copy.getBoundingClientRect()
  sheet.style.left = `${at.left - within.left - (now.left - at.left)}px`
  sheet.style.top = `${at.top - within.top - (now.top - at.top)}px`
  return frame
}

let leaving: Leaving | null = null

/**
 * The surface in `scope` has just drawn something else on its `data-stage`:
 * what it showed (`left`, from `capture`) is seen out as what it shows now is
 * seen in, as one movement.
 *
 * Going on to the next thing, the old sheet lifts away and the new one comes
 * up from under it; going back, the other way round — so the pane moves the
 * way the selection did, and stepping back undoes the step. A turn that is
 * not along the list (into the editor and out of it, the first thing shown)
 * has no direction: the new sheet steps forward into place.
 *
 * The sheet moves whole. What stands outside the stage — the pane's own
 * controls — does not move at all.
 *
 * `quick` is for a turn hard on the heels of another, a list stepped through
 * with a held key: the new sheet only, a third of the way, so each one can be
 * read as it passes and the run reads as a scroll.
 *
 * Nothing is left on the elements afterwards (no fill), so the stage does not
 * stay a stacking context that would hold its own overlays under the rest of
 * the window. Where the user asked for less motion there is none.
 */
export function turn(scope: ParentNode | null, left: Leaving | null, way: Way, quick = false) {
  // One sheet leaves at a time: a turn that arrives during another's exit
  // takes the stage from it.
  leaving?.remove()
  leaving = null

  const stage = stageOf(scope)
  if (!stage || still()) {
    left?.remove()
    return
  }

  if (quick) {
    left?.remove()
    stage.animate(
      [{ opacity: 0.45, transform: `translateY(${way * 7}px)` }, { opacity: 1, transform: 'none' }],
      { duration: 130, easing: 'ease-out' }
    )
    return
  }

  // Two animations a sheet, since its place and its ink keep different time:
  // it is moving for the whole turn and visible for only part of it.
  const from = way === 0 ? 'scale(0.97)' : `translateY(${way * TRAVEL}px)`
  stage.animate([{ transform: from }, { transform: 'none' }], { duration: TURN, easing: HOME })
  stage.animate([{ opacity: 0 }, { opacity: 1 }], {
    delay: HANDOVER * 0.6,
    duration: TURN * 0.5,
    easing: 'ease-out',
    fill: 'backwards'
  })

  const sheet = left?.firstElementChild
  if (!left || !sheet) return
  leaving = left
  if (way !== 0)
    sheet.animate([{ transform: 'none' }, { transform: `translateY(${-way * TRAVEL}px)` }], {
      duration: TURN * 0.6,
      easing: AWAY,
      fill: 'forwards'
    })
  sheet
    .animate([{ opacity: 1 }, { opacity: 0 }], { duration: HANDOVER, fill: 'forwards' })
    .finished.catch(() => {})
    .finally(() => {
      left.remove()
      if (leaving === left) leaving = null
    })
}
