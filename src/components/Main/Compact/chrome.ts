/**
 * iOS's minimum touch target, for the controls the desktop draws at 28-36px.
 * Passed to them as classes rather than asked for by a flag: it is the same
 * control, dressed for a finger.
 */
export const TOUCH = 'h-11 w-11'

/**
 * A tab root's header row: the title and its 44px actions centred on a 56px
 * row flush against the safe area — the very row a pushed screen's `NavBar`
 * puts its back control on (`BackButton` is `h-14`), so stepping from a root
 * into an entry and back moves nothing at the top of the screen. The roots
 * used to pad 16px above a 44px row and sat visibly lower than the screens
 * pushed over them.
 *
 * Side padding is the caller's: the list column pads its whole header box (the
 * row, the search and the chips) and the other roots pad the row itself.
 */
export const ROOT_HEADER = 'flex h-14 flex-none items-center gap-2.5'

/**
 * Where floating bottom chrome sits: the same 20px off every edge. Deliberately
 * not `env(safe-area-inset-bottom)` — that is UIKit's docked-content margin
 * (34pt on a home-indicator phone), and a pill lifted by it plus a gap floats
 * visibly higher than its side gutters. iOS's own floating tab bar sits a few
 * points above the home indicator (13pt from the edge), which 20px clears.
 */
const EDGE = 'inset-x-5 bottom-5'

/**
 * The floating tab bar's geometry, in the one place both it and the screens it
 * hovers over can read it: a 64px pill on [`EDGE`].
 */
export const TAB_BAR = `${EDGE} h-16`

/**
 * The Add disc nestled into the bar's top edge: a 56px accent circle centred
 * on that edge (20px offset + 64px bar − 28px radius = 56px from the bottom),
 * so half of it stands above the glass. The bar cuts a hole for it 6px wider
 * than the disc — [`TAB_BAR_NOTCH`] — so a ring of ground shows between the two
 * and the disc reads as its own object resting in the bar, not a fifth tab.
 */
export const ADD_DISC = 'bottom-14 h-14 w-14'
export const TAB_BAR_NOTCH =
  'mask-[radial-gradient(circle_at_50%_0,transparent_34px,#000_35px)]'

/**
 * The face of a screen pushed from the right — the entry, a settings pane, a
 * sub-page. It arrives across the whole width as the `Stack` layer under it
 * recedes, and throws a shadow off its left edge onto that layer: unseen at
 * rest, where it lies past the screen's edge, and the seam between the two
 * screens while an edge swipe holds them apart.
 */
export const PUSHED_SCREEN = 'flex min-h-0 flex-1 flex-col animate-push bg-screen shadow-push'

/**
 * What a root screen's scroller reserves under its content so the last row can
 * be scrolled clear of the bar it slides under (the disc's crown at 112px + a
 * breath). The fade behind the bar is the same height, so the disc always
 * stands on settled ground.
 */
export const TAB_BAR_CLEARANCE = 'pb-36'
export const TAB_BAR_FADE = 'h-36'

/**
 * The face of a screen's bottom action: an accent slab a thumb can land on
 * without aiming. Shared so the detail screen's Copy and the generator's Use
 * are visibly the same control, put on different footings below.
 */
export const ACTION_BUTTON =
  'absolute flex cursor-pointer items-center justify-center gap-2 rounded-xl bg-accent text-lg font-semibold text-accent-fg shadow-float transition-opacity disabled:cursor-default'

/**
 * A pushed screen's bottom action, on the same footing: a 54px button on
 * [`EDGE`] — the tab bar's own gutters, so the two never look like they were
 * measured by different people.
 */
export const PRIMARY_ACTION = `${EDGE} h-[54px]`

/** What that screen's scroller reserves under its content for the button. */
export const PRIMARY_CLEARANCE = 'pb-[102px]'

/**
 * The same action on a *root* screen, which keeps its tab bar: the button is
 * lifted clear of the Add disc's crown (its 112px + a 12px gap) rather than
 * sharing the bottom edge with the pill.
 */
export const ROOT_ACTION = 'z-10 inset-x-5 bottom-[124px] h-[54px]'

/** What that root's scroller reserves: the button, and the bar under it. */
export const ROOT_CLEARANCE = 'pb-[206px]'
