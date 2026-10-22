import type { CSSProperties, HTMLAttributes } from 'react'
import { cx } from '@/utils/cx'

/**
 * Which stock the face is cut from (see the `--paper-*`, `--slate-*` and
 * `--plastic-*` sets in theme.css). `paper` is fixed light: an ID document
 * stays paper when the app goes dark. `plastic` is fixed dark: the credit card
 * stays black plastic when the app is light. `slate` follows the theme: the
 * same paper on the light palette, and on the dark one a slab a shade off the
 * pane — the SSH fingerprint plate and the API key card are objects of the
 * app, not of the world, so they sit in the dark rather than glowing out of it.
 */
export type Tone = 'paper' | 'slate' | 'plastic'

// What a face is made of; every part comes from the tone's set, so the tones
// can only ever differ in colour, never in shape.
const PARTS = [
  'bg',
  'sheen',
  'edge',
  'shadow',
  'ink',
  'ink2',
  'rule',
  'tail',
  'hover',
  'bad'
] as const

// Whatever is printed on a face takes its inks from these — `text-(--face-ink2)`,
// `border-(--face-rule)`, `hover:bg-(--face-hover)` — and never a colour of its own.
const vars = (tone: Tone): CSSProperties =>
  Object.fromEntries(PARTS.map(part => [`--face-${part}`, `var(--${tone}-${part})`]))

/**
 * What everything pressable on a face wears: the hover wash in the face's own
 * ink, the small radius of an inlay, and a `group` for the copy hint to show
 * through (see `CopyHint`). The press itself is the caller's.
 */
export const PRESS =
  'group relative cursor-pointer rounded-sm text-left transition-colors hover:bg-(--face-hover)'

interface Props extends HTMLAttributes<HTMLDivElement> {
  tone?: Tone
}

/*
 * The surface every object in the vault is set on: an ID document, a credit
 * card, the SSH key's fingerprint plate, an API key's card. They are one kind
 * of thing on the page, so they share one face, whatever their size and what
 * is printed on it — a 16px radius, a 1px edge, a shadow just deep enough to
 * lift it off the pane, and a sheen from the top-right corner laid over the
 * printing. Every other attribute — a test id, a data attribute the e2e suite
 * reads — passes through.
 */
export default function Face({ tone = 'paper', className, children, ...rest }: Props) {
  return (
    <div
      {...rest}
      style={vars(tone)}
      className={cx(
        'relative overflow-hidden rounded-[16px] border border-(--face-edge) bg-(image:--face-bg) text-(--face-ink) tabular-nums shadow-(--face-shadow)',
        className
      )}
    >
      <div aria-hidden className="pointer-events-none absolute inset-0 z-10 bg-(image:--face-sheen)" />
      {children}
    </div>
  )
}
