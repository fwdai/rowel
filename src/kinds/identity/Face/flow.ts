// The printed fields, flowing beside the portrait and on under it — each as wide
// as its own caption needs and no wider, the way a document sets them. Not
// columns: a fixed column count is a decision about the page that the page has
// not asked for, and on a narrow pane it is the thing that starts dictating how
// tall the document has to be.
//
// The flow sizes itself and the data area takes its height from that. It must
// not be handed a height to fit into — a wrapping flex container asked to fit a
// box shorter than its lines lays them out anyway and spills over whatever is
// below, and its intrinsic height does not travel back up through a `flex-1`
// parent to stop that happening. So nothing above it stretches, and the slack on
// a document with room to spare is collected above its foot instead.
export const FIELDS = 'flex min-w-0 flex-1 flex-wrap content-start gap-x-5 gap-y-2.5'

// The holder's name: the biggest thing a document prints about its holder.
export const NAME = 'text-lg font-medium uppercase tracking-[0.03em]'

// A card's masthead: the row, cleared on the right for the mark in the corner
// (see `Medallion`), its title line, and the quieter line under that.
export const HEAD = 'flex h-9 flex-none items-center gap-3 pr-16'
export const TITLE = 'truncate text-base font-semibold uppercase leading-tight tracking-label'
export const SUBTITLE = 'truncate text-2xs uppercase tracking-[0.1em] text-(--face-ink2)'
