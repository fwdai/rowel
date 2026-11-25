import { cx } from '@/utils/cx'
import { CARD, GROUND } from './ground'

interface Props {
  /** The tint the stock is washed in: a CSS gradient. */
  wash: string
  /** The ink the ground is drawn in. */
  line: string
  /** The card's other side: the same printing, seen from behind. */
  mirrored?: boolean
}

/**
 * A card's security printing: the wash of colour its stock is tinted with and
 * a fine-line ground over that (see `ground.ts`), across the whole face and
 * under everything printed on the card (the face's frame is what it is pinned
 * to, see `Frame`). The colour is what tells one card from another; the ground
 * is only there to make it paper. The paper itself is the vault's own (`Face`,
 * in its paper tone).
 *
 * Fixed colours, as the rest of a document face: the card stays the colour it
 * is whichever palette the app is on.
 */
export default function Print({ wash, line, mirrored }: Props) {
  return (
    <div
      aria-hidden
      className={cx('pointer-events-none absolute inset-0 -z-10', mirrored && '-scale-x-100')}
      style={{ background: wash }}
    >
      <svg
        viewBox={`0 0 ${CARD.width} ${CARD.height}`}
        preserveAspectRatio="xMidYMid slice"
        className="h-full w-full"
      >
        <path d={GROUND} fill="none" stroke={line} strokeWidth={0.5} opacity={0.2} />
      </svg>
    </div>
  )
}
