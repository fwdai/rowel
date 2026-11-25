import type { ReactNode } from 'react'
import { FingerprintGlyph, LicenseGlyph } from '@/components/Main/icons'
import type { Doc } from '../DocCell'
import Hologram from '../Hologram'

/**
 * What a card is made of, the same on both its sides: the wash its stock is
 * tinted with and the ink its ground is drawn in (see `Print`), and the card's
 * mark — small in the corner of its front, large and faint in the stock of its
 * back.
 */
export interface Stock {
  wash: string
  line: string
  mark: (size: number, doc: Doc) => ReactNode
}

// A mark drawn in line holds one hairline at either size.
const HAIRLINE = 1.5

/** Rose, the colour the card has had since it was paper. */
export const LICENSE: Stock = {
  wash: 'linear-gradient(118deg, rgba(226, 124, 160, 0.26), rgba(226, 124, 160, 0.09) 52%, rgba(122, 146, 226, 0.2))',
  line: '#C2527F',
  mark: size => <LicenseGlyph size={size} stroke={HAIRLINE} />
}

/** Blue, with the issuing state's emblem in film. */
export const ID_CARD: Stock = {
  wash: 'linear-gradient(118deg, rgba(92, 138, 214, 0.24), rgba(92, 138, 214, 0.08) 52%, rgba(74, 184, 172, 0.2))',
  line: '#4A7AC0',
  mark: (size, doc) => <Hologram country={doc.value('country')} size={size} />
}

/** Green. */
export const PERMIT: Stock = {
  wash: 'linear-gradient(118deg, rgba(70, 168, 126, 0.26), rgba(70, 168, 126, 0.09) 52%, rgba(216, 176, 98, 0.2))',
  line: '#2F8A66',
  mark: size => <FingerprintGlyph size={size} stroke={HAIRLINE} />
}
