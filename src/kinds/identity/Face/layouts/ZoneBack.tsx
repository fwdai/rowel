import Eye from '@/components/elements/Face/Eye'
import MachineZone from '../MachineZone'
import type { BackProps } from '.'
import Reverse, { Watermark } from './Reverse'

/**
 * The back of a card that travels — an ID card, a residence permit: the card's
 * mark large in the stock, and the machine-readable zone across its lower
 * third, three lines of thirty generated from the front, where a border gate
 * reads it. The eye is the front's: the zone repeats the numbers the front
 * masks, and stays masked with them.
 */
export default function ZoneBack({ docType, doc, stock }: BackProps) {
  return (
    <Reverse docType={docType} doc={doc} band={<MachineZone docType={docType} doc={doc} card />}>
      <Watermark stock={stock} doc={doc} className="mr-auto" />
      <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-band" />
    </Reverse>
  )
}
