import type { Ref } from 'react'
import { cx } from '@/utils/cx'
import AsteriskGlyph from '../AsteriskGlyph'

// Cell pitch in px: a hair under the 15px type size so the row reads gently
// tracked-out without going sparse. Exported so the input can map a click's x
// back to a character index against the same grid it is drawn on.
export const CELL = 15

// Mask glyph size in px. Two thirds of the cell: the asterisk's notches make
// it lighter than a disc of the same box, so it lands near the weight of a
// revealed character and the row doesn't jump on reveal.
const MASK = 10

// Stagger offsets for the busy ripple, precomputed so renders allocate no
// style objects. 40 covers any plausible passphrase; longer ones wrap.
const WAVE_DELAYS = Array.from({ length: 40 }, (_, i) => ({
  animationDelay: `${i * 70}ms`
}))

interface Props {
  count: number
  caret: boolean
  // The input's live [selectionStart, selectionEnd]; the caret is drawn at the
  // start boundary and the cells in between are washed as selected.
  selection: [number, number]
  // Attached to the cell row so the input can read its on-screen geometry.
  rowRef?: Ref<HTMLDivElement>
  // The value to show in place of the dots (reveal mode). Same cell grid, so
  // each character appears exactly where its dot was — a true in-place reveal.
  text?: string
  // Verifying: the cells ripple in sequence while the key derives, the
  // immediate "we heard you" feedback for the deliberately slow unlock.
  busy?: boolean
}

// One fixed-width cell per typed character. Cells hold either a mask glyph
// (the brand asterisk, the mascot's own body: a masked character is what it
// is) or, when `text` is given, the character itself — identical geometry
// either way, centered over the (text-transparent) input. The caret is drawn
// on the cell boundary matching the input's real selection, so editing
// mid-passphrase shows the bar exactly where backspace will act.
export default function Dots({
  count,
  caret,
  selection: [start, end],
  rowRef,
  text,
  busy
}: Props) {
  return (
    <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
      <div ref={rowRef} className="relative flex">
        {Array.from({ length: count }).map((_, i) => (
          <span
            key={i}
            className={cx(
              'animate-fade flex flex-none items-center justify-center rounded-xs',
              i >= start && i < end && 'bg-lockfield-soft'
            )}
            style={{ width: CELL }}
          >
            <span
              className={cx(
                'flex',
                busy && 'animate-[dotwave_1.05s_ease-in-out_infinite]',
                text === undefined ? 'text-text/75' : 'text-md text-text'
              )}
              style={busy ? WAVE_DELAYS[i % WAVE_DELAYS.length] : undefined}
            >
              {text === undefined ? <AsteriskGlyph size={MASK} /> : text[i]}
            </span>
          </span>
        ))}
        {caret && (
          <span
            className="absolute top-1/2 h-6 w-px -translate-x-1/2 -translate-y-1/2 bg-lockfield animate-[caret_1.1s_steps(1,end)_infinite]"
            style={{ left: start * CELL }}
          />
        )}
      </div>
    </div>
  )
}
