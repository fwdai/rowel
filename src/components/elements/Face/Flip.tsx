import { useEffect, useRef, type ReactNode } from 'react'
import { cx } from '@/utils/cx'

interface Props {
  /** Whether the back is the side showing. */
  flipped: boolean
  front: ReactNode
  back: ReactNode
  /** The pair's box: both sides fill it and take the taller one's height. */
  className?: string
}

// Either side of the turning card: both share one grid cell, so they are the
// same size whichever holds more, and each hides when it faces away.
const SIDE = 'col-start-1 row-start-1 grid min-w-0 backface-hidden'

/**
 * A face with a back: the two sides of one object, turned over about its
 * vertical axis the way a card is turned in the hand. Both sides stay mounted
 * so the turn has something to show on its way round; the one facing away is
 * `inert`, out of the tab order and the accessibility tree until it comes back.
 * With motion reduced the sides simply change places.
 *
 * Going inert takes the focus off whatever turned the face, so after a turn it
 * is handed to the control that turns it back: the element each side marks
 * `data-flip`. A keyboard user stays on the card instead of at the top of the
 * page.
 */
export default function Flip({ flipped, front, back, className }: Props) {
  const sides = useRef<HTMLDivElement>(null)
  const was = useRef(flipped)

  useEffect(() => {
    if (was.current === flipped) return
    was.current = flipped
    sides.current
      ?.querySelector<HTMLElement>(':scope > :not([inert]) [data-flip]')
      ?.focus({ preventScroll: true })
  }, [flipped])

  return (
    <div className={cx('perspective-[1600px]', className)}>
      <div
        ref={sides}
        className={cx(
          'grid transition-transform duration-500 transform-3d motion-reduce:transition-none',
          flipped && 'rotate-y-180'
        )}
      >
        <div className={SIDE} inert={flipped}>
          {front}
        </div>
        <div className={cx(SIDE, 'rotate-y-180')} inert={!flipped}>
          {back}
        </div>
      </div>
    </div>
  )
}
