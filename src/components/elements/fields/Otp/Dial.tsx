import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { useCopied } from '@/hooks/useCopied'
import { CheckGlyph, CopyGlyph } from '@/components/Main/icons'
import { META } from '../../tokens'
import { groupDigits } from './secret'

// A compact dial: the six usual digits at `text-md` need ~70px of clear space,
// and the ring leaves ~80px inside its stroke, so it sits around them rather
// than cutting through the first and last.
const SIZE = 96
const STROKE = 5
const R = SIZE / 2 - STROKE
const CIRCUMFERENCE = 2 * Math.PI * R
// The disc the overlays fill: just inside the ring, with a hair of ground
// between the two so the disc reads as sitting in the ring, not as the ring.
const DISC_INSET = STROKE * 2

// The code, and how long it lives, as one object: a countdown ring around the
// digits. Used to show a saved secret's code and to preview a typed one.
// `period` is the seed's own window, not always 30s, so the ring empties in
// step with the code rather than at a rate of its own.
//
// `copyable` makes the whole dial the copy control: hovering swaps the digits
// for a copy glyph, and a click copies the code and flashes a check in the same
// place. The editor's preview is not copyable — while typing, the dial is proof
// the secret works, not a thing to take the code from.
export default function Dial({
  code,
  time,
  period,
  copyable
}: {
  code: string
  time: number
  period: number
  copyable?: boolean
}) {
  const { t } = useTranslation()
  const { copied, copy } = useCopied()
  const digits = groupDigits(code)

  const ring = (
    // The dash starts at 3 o'clock and runs clockwise. Mirroring the drawing
    // and turning it a quarter puts the start at 12 and lays the remaining arc
    // out counter-clockwise from there — so its free end, the one that moves,
    // sweeps clockwise as the window runs out, the way a clock hand does.
    <svg
      width={SIZE}
      height={SIZE}
      viewBox={`0 0 ${SIZE} ${SIZE}`}
      fill="none"
      className="absolute inset-0 rotate-90 -scale-x-100"
    >
      <circle cx={SIZE / 2} cy={SIZE / 2} r={R} stroke="var(--c-line)" strokeWidth={STROKE} />
      <circle
        cx={SIZE / 2}
        cy={SIZE / 2}
        r={R}
        stroke="var(--c-accent)"
        strokeWidth={STROKE}
        strokeLinecap="round"
        strokeDasharray={`${(time / period) * CIRCUMFERENCE} ${CIRCUMFERENCE}`}
      />
    </svg>
  )

  const face = (
    // Heavy, so the code reads at a glance, and tracked at half the secret
    // spacing: enough air to tell the digits apart, compact enough to sit well
    // inside the ring. An 8- or 10-digit seed steps the type down and drops the
    // spacing so it stays clear of the ring at this diameter.
    <div
      className={cx(
        'font-semibold tabular-nums text-text transition-opacity',
        code.length > 6 ? 'text-base' : 'text-md tracking-[0.04em]',
        // The digits recede under the overlay rather than vanish, so the disc
        // reads as a veil over the code. Only a pointer that can hover gets
        // this: on touch the digits stay put, and a tap flashes the check.
        copyable &&
          'group-hover:opacity-25 group-focus-visible:opacity-25 any-pointer-coarse:group-hover:opacity-100',
        copied && 'opacity-25'
      )}
    >
      {digits}
    </div>
  )

  const frame = 'relative m-[10px_0_4px] grid place-items-center'
  const size = { width: SIZE, height: SIZE }

  return (
    <>
      {copyable ? (
        <button
          type="button"
          onClick={() => copy(code)}
          title={t('Copy code')}
          aria-label={t('Copy code')}
          className={`group cursor-pointer rounded-full outline-none ${frame}`}
          style={size}
        >
          {ring}
          {face}
          {/* One veil at a time: a hint of the hover tint under the copy glyph
              until a click, then a wash of the success green under the check.
              The pointer is still on the dial after the click, so the hover
              disc has to go rather than be painted over. */}
          {copied ? (
            <span
              className="pointer-events-none absolute grid place-items-center rounded-full bg-good/15 text-good"
              style={{ inset: DISC_INSET }}
            >
              <CheckGlyph size={20} />
            </span>
          ) : (
            <span
              className="pointer-events-none absolute grid place-items-center rounded-full bg-hover text-text2 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100 any-pointer-coarse:group-hover:opacity-0"
              style={{ inset: DISC_INSET }}
            >
              <CopyGlyph size={20} />
            </span>
          )}
        </button>
      ) : (
        <div className={frame} style={size}>
          {ring}
          {face}
        </div>
      )}
      <div className={META}>{t('refreshes in {{n}}s', { n: time })}</div>
    </>
  )
}
