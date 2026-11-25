import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { RefreshGlyph } from '@/components/Main/icons'
import { cx } from '@/utils/cx'
import { rosette } from './guilloche'

const SIZE = 60

const RING =
  rosette({ x: SIZE / 2, y: SIZE / 2, radius: 23, depth: 6, lobes: 10, strands: 6 }) +
  rosette({ x: SIZE / 2, y: SIZE / 2, radius: 17, depth: 1.5, lobes: 20, strands: 2 })

const PLACE = 'absolute right-4 top-2.5 grid place-items-center'

interface Props {
  /** The card's engraving ink: the ring's, and the mark's unless it brings its own. */
  line: string
  /** Turns the card over. Given, the mark is the control that does it. */
  onFlip?: () => void
  children: ReactNode
}

/**
 * The card's mark, in its top-right corner — the place a credit card wears its
 * network's: a licence's car, an ID card's national emblem, a permit's
 * fingerprint, each set in an engraved ring. It is the one glance that says
 * which card this is, and it sits where the face catches the light.
 *
 * It is also what turns the card over. The mark is in the same corner on both
 * sides, so the control stays under the pointer through a turn and back, and a
 * small badge on its rim says it can be pressed. A card that cannot be turned —
 * one being edited, whose back has nothing to type — wears the mark alone.
 */
export default function Medallion({ line, onFlip, children }: Props) {
  const { t } = useTranslation()
  const mark = (
    <>
      <svg
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        className="absolute inset-0 h-full w-full transition-transform duration-500 group-hover:rotate-[36deg]"
      >
        <path d={RING} fill="none" stroke="currentColor" strokeWidth={0.6} />
      </svg>
      <div className="relative">{children}</div>
    </>
  )
  const box = { width: SIZE, height: SIZE, color: line }

  if (!onFlip)
    return (
      <div aria-hidden className={cx(PLACE, 'pointer-events-none')} style={box}>
        {mark}
      </div>
    )

  return (
    <button
      type="button"
      title={t('Flip card')}
      aria-label={t('Flip card')}
      data-testid="flip-card"
      onClick={onFlip}
      className={cx(PLACE, 'group cursor-pointer rounded-full')}
      style={box}
    >
      {mark}
      <span className="absolute bottom-0.5 right-0.5 grid h-4 w-4 place-items-center rounded-full bg-white text-(--face-ink2) shadow-[0_0_0_1px_var(--face-rule),0_1px_2px_rgba(20,22,26,0.12)] transition-colors group-hover:text-(--face-ink)">
        <RefreshGlyph size={10} />
      </span>
    </button>
  )
}
