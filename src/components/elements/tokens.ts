import type { CSSProperties } from 'react'

// A step rounder on the phone shell, where a card is the screen's main shape
// rather than a panel inside a pane. `shadow-card` is its lift off the pane:
// a faint drop plus a white top edge on light, a top glow alone on dark.
export const CARD =
  'overflow-hidden rounded-lg border border-line bg-card shadow-card max-md:rounded-xl'

export const ROW_HAIRLINE = 'inset-shadow-hairline last:inset-shadow-none'

// A 460px face (a credit card, an ID document) with its note panel: stacked,
// and side by side once the *pane* is wide enough for both — 460 plus a note
// column worth reading. A container breakpoint rather than a viewport one:
// the detail pane can be far narrower than the window (a split view, an iPad
// half), and the faces themselves fold by container width, so the two must
// agree on what "wide" means.
export const FACE_ASIDE =
  'grid grid-cols-1 items-start gap-3 @min-[720px]:grid-cols-[460px_minmax(0,1fr)]'

// The wide shell's detail pane inset, and its exact negative: a surface that
// wants the whole pane (an empty state with a watermark run off the corner)
// takes PANE_BLEED to undo PANE_PAD, so the two must always move together.
export const PANE_PAD = 'pt-[26px] px-[34px] pb-[60px]'
export const PANE_BLEED = 'md:-mt-[26px] md:-mx-[34px] md:-mb-[60px]'

// A trailing control that stays out of the way until the row is asked about —
// hovered, or holding the keyboard. Pairs with a `group` on the row itself.
// Opacity only: the control keeps its place in the layout and in the tab order.
// A finger cannot hover, so on touch the control is simply always there.
// `any-pointer-coarse`, not `pointer-coarse`: the latter only matches when the
// *primary* pointer is coarse, which leaves an iPad with a trackpad or a touch
// laptop hiding the control from the finger that is also on the device.
export const HOVER_ONLY =
  'opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 any-pointer-coarse:opacity-100'

// A row's copy button, which the phone does without: there the value is the
// row's copy control (`fields/CopyValue`), its own check confirms a press, and the
// screen's bottom button covers the main secret — as Passwords and 1Password
// draw their rows. Every row that offers a copy button wraps it in this, so a
// phone rail holds only what means something else: the eye, the open-link.
export const ROW_COPY = 'max-md:hidden'

// App-level transient feedback (the update prompt, the scan status): a floating
// panel on the detail surface. Each toast places itself — two of them in the
// same corner would sit on top of each other.
export const TOAST =
  'animate-pop fixed z-[1000] max-w-[340px] rounded-xl border border-line bg-detail text-text shadow-float'

// The micro-label face, without an ink: 11px, uppercase, tracked, regular
// weight — a label is the secondary line, and the value it captions carries the
// weight. Take this when the label needs a different colour (the accent
// "EDITING ·" eyebrow) and LABEL otherwise.
export const LABEL_TYPE = 'text-xs uppercase tracking-label'

// Small text sits on the secondary tier (text2), not the tertiary: text3 is
// ~3.6:1 on white and ~2.9:1 on the app ground in light mode, under WCAG AA's
// 4.5:1 for text — fine for a glyph (3:1), too pale for an 11px label or stamp
// (see Compact/Tab, which measured the same and moved first).
export const LABEL = `${LABEL_TYPE} text-text2`

// The label over a detail row's value. On the phone it is the grouped list's
// caption — sentence case, 12px, the secondary ink, no tracking — rather than
// the desktop's tracked uppercase: at 11px uppercase and 0.12em the label read
// as big as the 16px value it captions, and a row became two lines of the
// same weight.
export const ROW_LABEL = `${LABEL} max-md:text-sm max-md:normal-case max-md:tracking-[0]`

// The one gutter for everything that stands on the pane outside a card — a
// plain list of files and its heading, an "add" verb under a section, a note
// under either. 4px in from the cards' edge: a glyph or a capital set dead on
// the cards' line reads as starting before it, because the card's stroke and
// corner sit inside its box while the ink runs to the edge of its. Applied
// once, as a wrapper, so the things inside it carry no inset of their own and
// cannot drift apart from each other.
export const EDGE_GUTTER = 'px-1'

// A section's heading over plain rows (the files, the tags): the label face, set off
// what it heads by its own gap. On the phone it is the grouped list's section
// header — still uppercase, as iOS sets them, but at 12px with the tracking
// pulled in. It carries no inset of its own: it stands inside `EDGE_GUTTER`
// with the rows it heads.
export const SECTION_LABEL_FLUSH = `mb-1.5 block ${LABEL} max-md:mb-2 max-md:text-sm max-md:tracking-[0.06em]`

// The same heading over a card (passkeys, custom fields): inset to the
// card's own 16px, so it lines up with the captions inside, as iOS aligns a
// group's header with its cells' text.
export const SECTION_LABEL = `${SECTION_LABEL_FLUSH} max-md:px-4`

// An "add" action standing under a section ("Attach file"): the accent text
// line `AddAction` draws, on both shells. On the phone it takes a 44px row
// with the row tier's type and an 18px mark, so it is a finger's target
// without becoming a thing; it stands inside `EDGE_GUTTER` and carries no
// inset of its own. It used to be a card cell of its own, which put a boxed
// button under every section and made the screen read as a stack of cards
// rather than as content with quiet verbs. Not a tag's: "Add tag" is a chip
// among the chips (`TagChip`), the shape of what it adds.
export const ACTION_ROW = 'max-md:h-11 max-md:gap-2.5 max-md:text-md max-md:[&>svg]:size-[18px]'

// The meta face, without an ink: counts, timestamps, hints, shortcuts, chips —
// the same 11px as the label tier, set as ordinary text rather than a tracked
// uppercase eyebrow. Tabular figures, so a count or a countdown holds its width
// as it changes instead of nudging what sits beside it. Take this when the ink
// is the caller's (a count inheriting its chip's colour, an error in `text-bad`)
// and META otherwise.
export const META_TYPE = 'text-xs tabular-nums'

export const META = `${META_TYPE} text-text2`

// The detail row's value line: one line high, never wrapping, the ink the
// caller's. VALUE_LINE leaves the size open for the one row that sets its own
// (the headline secret, at text-xl); everything else takes VALUE.
export const VALUE_LINE = 'block h-6 min-w-0 truncate leading-6'

export const VALUE = `${VALUE_LINE} text-base`

// What a masked secret is read as. Fixed, so the mask says nothing about the
// secret's length: twelve dots for a value line, twenty-four for a block.
export const MASK_DOTS = '•'.repeat(12)
export const BLOCK_DOTS = '•'.repeat(24)

// An input cannot fake dots, so a masked editor box hides its own text instead.
export const MASK_INPUT = { WebkitTextSecurity: 'disc' } as CSSProperties
