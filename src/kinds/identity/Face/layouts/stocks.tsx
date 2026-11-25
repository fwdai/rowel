import type { ReactNode } from 'react'
import { FingerprintGlyph, LicenseGlyph } from '@/components/Main/icons'
import type { Doc } from '../DocCell'
import Hologram from '../Hologram'
import { ribbon, rosette } from '../guilloche'

/**
 * What a card is made of, the same on both its sides: the wash its stock is
 * tinted with, the engraving over that and the ink it is cut in (see `Print`),
 * and the card's mark — small in the corner of each side, large in the stock of
 * its back.
 */
export interface Stock {
  wash: string
  line: string
  engraving: string
  mark: (size: number, doc: Doc) => ReactNode
}

// A mark drawn in line holds one hairline at either size.
const HAIRLINE = 1.5

/** Rose, the colour the card has had since it was paper, under two ribbons. */
export const LICENSE: Stock = {
  wash: 'linear-gradient(118deg, rgba(226, 124, 160, 0.26), rgba(226, 124, 160, 0.09) 52%, rgba(122, 146, 226, 0.2))',
  line: '#C2527F',
  engraving:
    ribbon({ y: 84, depth: 20, length: 104, strands: 6, swell: 0.35 }) +
    ribbon({ y: 236, depth: 24, length: 138, strands: 7, swell: -0.3 }),
  mark: size => <LicenseGlyph size={size} stroke={HAIRLINE} />
}

/** Blue, a medallion woven behind the holder's details, the state's emblem in film. */
export const ID_CARD: Stock = {
  wash: 'linear-gradient(118deg, rgba(92, 138, 214, 0.24), rgba(92, 138, 214, 0.08) 52%, rgba(74, 184, 172, 0.2))',
  line: '#4A7AC0',
  engraving:
    rosette({ x: 322, y: 172, radius: 104, depth: 24, lobes: 14, strands: 7 }) +
    rosette({ x: 322, y: 172, radius: 52, depth: 14, lobes: 9, strands: 6 }) +
    rosette({ x: 36, y: 268, radius: 64, depth: 13, lobes: 12, strands: 5 }),
  mark: (size, doc) => <Hologram country={doc.value('country')} size={size} />
}

/** Green, a ribbon across it and a medallion run off its corner. */
export const PERMIT: Stock = {
  wash: 'linear-gradient(118deg, rgba(70, 168, 126, 0.26), rgba(70, 168, 126, 0.09) 52%, rgba(216, 176, 98, 0.2))',
  line: '#2F8A66',
  engraving:
    rosette({ x: 418, y: 250, radius: 88, depth: 20, lobes: 13, strands: 6 }) +
    ribbon({ y: 118, depth: 26, length: 150, strands: 7, swell: 0.3 }),
  mark: size => <FingerprintGlyph size={size} stroke={HAIRLINE} />
}
