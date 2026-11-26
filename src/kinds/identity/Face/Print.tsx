import { useId } from 'react'
import { lineOf, washOf, type CardColor } from './colors'

/**
 * A card's security printing: the wash of colour its stock is tinted with and
 * a fine-line ground over that, across the whole face and under everything
 * printed on the card (the face's frame is what it is pinned to, see `Frame`).
 * The colour is what tells one card from another; the ground is only there to
 * make it paper — one shallow wave, repeated, each row sheared a little along
 * from the one above. Texture rather than artwork: no figure in it that a real
 * card would have to match.
 *
 * Fixed colours, as the rest of a document face: the card stays the colour it
 * is whichever palette the app is on.
 */
export default function Print({ color }: { color: CardColor }) {
  const ground = useId()

  return (
    <svg
      aria-hidden
      className="pointer-events-none absolute inset-0 -z-10 h-full w-full"
      style={{ background: washOf(color), color: lineOf(color) }}
    >
      <pattern
        id={ground}
        width="150"
        height="9"
        patternUnits="userSpaceOnUse"
        patternTransform="skewX(55)"
      >
        <path d="M0 4.5q37.5-7 75 0t75 0" fill="none" stroke="currentColor" strokeWidth="0.5" />
      </pattern>
      <rect width="100%" height="100%" fill={`url(#${ground})`} opacity="0.2" />
    </svg>
  )
}
