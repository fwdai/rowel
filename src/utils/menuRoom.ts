import { scrollParent } from './scrollParent'

// The breath a menu keeps from the edge it is measured against.
const MARGIN = 8

export interface MenuRoom {
  /** Open above the trigger: that is the side with more room. */
  up: boolean
  /** The height there is on that side, in px, to cap the menu to. */
  room: number
}

/**
 * Which side of `trigger` a menu hung off it should open to, and how tall it
 * may be there. Measured against what the menu must stay inside: the nearest
 * ancestor that scrolls (the settings modal's section scroller, the phone's
 * pane), else the window. A menu that runs past a scroller's edge is cut off
 * or left under the fold, so the window's room is not the room there is.
 */
export const menuRoom = (trigger: HTMLElement): MenuRoom => {
  const { top, bottom } = trigger.getBoundingClientRect()
  const bounds = scrollParent(trigger)?.getBoundingClientRect() ?? {
    top: 0,
    bottom: window.innerHeight
  }
  const above = top - bounds.top
  const below = bounds.bottom - bottom
  return { up: above > below, room: Math.max(above, below) - MARGIN }
}
