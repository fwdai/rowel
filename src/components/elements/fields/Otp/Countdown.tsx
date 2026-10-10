import type { CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'

// How long the code on the row beside it still has, as a ring in the rail:
// the accent arc is what is left of the window, and its free end sweeps
// clockwise as the window runs out, the way a clock hand does — the same
// motion as the dial's, in a quarter of the room. Drawn with a conic gradient
// rather than an SVG, so the rail's glyph sizing leaves it alone; `--otp-left`
// is registered in `theme.css`, so the second-by-second steps the generator
// hands over glide into each other. Keyed on the code: a fresh window starts
// full at once rather than winding back up.
export default function Countdown({
  code,
  time,
  period
}: {
  code: string
  time: number
  period: number
}) {
  const { t } = useTranslation()
  const label = t('refreshes in {{n}}s', { n: time })

  return (
    // A rail slot — 28px on the desktop, the 44px target on the phone — with
    // the ring itself a step inside it.
    <span className="grid h-7 w-7 flex-none place-items-center max-md:h-11 max-md:w-11">
      <span
        key={code}
        role="img"
        aria-label={label}
        title={label}
        data-testid="otp-countdown"
        className="otp-ring block h-[22px] w-[22px] rounded-full"
        style={{ '--otp-left': `${(time / period) * 100}%` } as CSSProperties}
      />
    </span>
  )
}
