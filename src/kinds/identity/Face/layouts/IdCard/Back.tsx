import Eye from '@/components/elements/Face/Eye'
import Chip from '../../Chip'
import Hologram from '../../Hologram'
import MachineZone from '../../MachineZone'
import Reverse from '../Reverse'
import type { LayoutProps } from '..'
import { STOCK } from './stock'

/**
 * The back of an identity card: the chip's contacts, the state's emblem large
 * in the film over the stock, and the machine-readable zone — three lines of
 * thirty, generated from the front — across the card's lower third, where a
 * border gate reads it. The eye is the front's: the zone repeats the two
 * numbers the front masks, and stays masked with them.
 */
export default function IdCardBack(props: LayoutProps) {
  const { docType, doc } = props
  const country = doc.value('country')

  return (
    <Reverse
      {...props}
      stock={STOCK}
      mark={<Hologram country={country} className="h-7 w-7" />}
      band={<MachineZone docType={docType} doc={doc} card />}
    >
      <Chip className="h-9 w-auto flex-none" />
      <Hologram country={country} className="mx-auto h-24 w-24 flex-none opacity-60" />
      <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-band" />
    </Reverse>
  )
}
