import type { CSSProperties } from 'react'
import { LABEL_TYPE } from '@/components/elements/tokens'
import DocCell, { type Doc } from './DocCell'

// The plate is dark whatever it is set on, so the cell inside it takes light
// inks — the same `--face-*` parts every cell reads, re-cut for this island.
const INKS = {
  '--face-ink': '#FFFFFF',
  '--face-ink2': 'rgba(255, 255, 255, 0.6)',
  '--face-rule': 'rgba(255, 255, 255, 0.4)',
  '--face-hover': 'rgba(255, 255, 255, 0.16)',
  '--face-bad': '#FFB4AB'
} as CSSProperties

/**
 * The issuing country's code on its plate, in the card's top-left corner —
 * where a licence and an ID card both carry the distinguishing sign of the
 * state that issued them. Editing, the plate is where the code is typed.
 * Reading a card with no country, there is no plate.
 */
export default function Plate({ doc }: { doc: Doc }) {
  if (!doc.set && !doc.value('country')) return null

  return (
    <div
      style={INKS}
      className="grid h-9 min-w-11 flex-none place-items-center rounded-[5px] bg-[#27468C] px-2 shadow-[inset_0_0_0_1px_rgba(255,255,255,0.18)]"
    >
      <DocCell
        doc={doc}
        name="country"
        bare
        ink={`${LABEL_TYPE} font-semibold text-(--face-ink)`}
        size={4}
      />
    </div>
  )
}
