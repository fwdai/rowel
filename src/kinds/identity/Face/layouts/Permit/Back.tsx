import Eye from '@/components/elements/Face/Eye'
import { FingerprintGlyph } from '@/components/Main/icons'
import Chip from '../../Chip'
import MachineZone from '../../MachineZone'
import Reverse from '../Reverse'
import type { LayoutProps } from '..'
import { STOCK } from './stock'

/**
 * The back of a residence permit: the chip's contacts, the print of a finger
 * large in the stock — what the chip holds — and the machine-readable zone
 * across the card's lower third, under the permit's own document code. The eye
 * is the front's: the zone repeats the number the front masks.
 */
export default function PermitBack(props: LayoutProps) {
  const { docType, doc } = props

  return (
    <Reverse
      {...props}
      stock={STOCK}
      mark={<FingerprintGlyph size={24} />}
      band={<MachineZone docType={docType} doc={doc} card />}
    >
      <Chip className="h-9 w-auto flex-none" />
      <div aria-hidden className="mx-auto flex-none opacity-30" style={{ color: STOCK.line }}>
        <FingerprintGlyph size={92} stroke={1.5} />
      </div>
      <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-band" />
    </Reverse>
  )
}
