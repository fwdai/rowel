import { useState, type ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { FlipGlyph } from '@/components/Main/icons'

interface Props {
  /** Whether the back is the side showing. */
  flipped: boolean
  /** Turns the face over. Left off, the face shows the side it is on and offers no turn. */
  onFlip?: () => void
  /** What the control that turns it is called, already translated. */
  label: string
  front: ReactNode
  back: ReactNode
  /** The pair's box: both sides fill it and take the taller one's height. */
  className?: string
}

// Either side of the turning card: both share one grid cell, so they are the
// same size whichever holds more, and each hides when it faces away.
const SIDE = 'col-start-1 row-start-1 grid min-w-0 backface-hidden'

// The key that turns the face. White on every palette, in the paper's inks: it
// rides the edge of an object that keeps its own colours when the app goes
// dark, so it keeps them too — a theme-coloured key would sink into the pane on
// one side of it. Where a finger is one of the pointers the hit area grows to
// the 44px target through a pseudo-element, as the face's eye does.
const KEY =
  "absolute -right-2.5 top-1/2 z-20 grid h-6 w-6 -translate-y-1/2 cursor-pointer place-items-center rounded-full bg-white text-(--paper-ink2) shadow-[0_0_0_1px_rgba(20,22,26,0.1),0_1px_3px_rgba(20,22,26,0.25)] transition-colors hover:text-(--paper-ink) any-pointer-coarse:before:absolute any-pointer-coarse:before:-inset-2.5 any-pointer-coarse:before:content-['']"

// Where the face rests: flat on the side it shows, or lifted a little towards
// the other one while the pointer is on the control that turns it.
const REST = { front: 'rotate-y-0', back: 'rotate-y-180' }
const LIFTED = { front: 'rotate-y-[14deg]', back: 'rotate-y-[166deg]' }

/**
 * A face with a back: the two sides of one object, turned over about its
 * vertical axis the way a card is turned in the hand. Both sides stay mounted
 * so the turn has something to show on its way round; the one facing away is
 * `inert`, out of the tab order and the accessibility tree until it comes back.
 * With motion reduced the sides simply change places.
 *
 * What turns it is a small raised key on the face's right edge — the edge it
 * turns about. It stands outside both sides: nothing printed on the face makes
 * room for it, it does not go inert with the side that was showing, and it is
 * in the same place whichever side is up. Under the pointer it lifts the face a
 * few degrees, which says what it does before it is pressed.
 */
export default function Flip({ flipped, onFlip, label, front, back, className }: Props) {
  const [lifted, setLifted] = useState(false)
  const side = flipped ? 'back' : 'front'

  return (
    <div className={cx('relative perspective-[1600px]', className)}>
      <div
        className={cx(
          'grid transition-transform duration-500 transform-3d motion-reduce:transition-none',
          (lifted && onFlip ? LIFTED : REST)[side]
        )}
      >
        <div className={SIDE} inert={flipped}>
          {front}
        </div>
        <div className={cx(SIDE, 'rotate-y-180')} inert={!flipped}>
          {back}
        </div>
      </div>

      {onFlip && (
        <button
          type="button"
          title={label}
          aria-label={label}
          aria-pressed={flipped}
          data-testid="flip-face"
          onClick={onFlip}
          onPointerEnter={() => setLifted(true)}
          onPointerLeave={() => setLifted(false)}
          className={KEY}
        >
          <FlipGlyph size={13} />
        </button>
      )}
    </div>
  )
}
