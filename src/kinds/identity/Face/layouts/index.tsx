import type { ComponentType, ReactNode } from 'react'
import { FingerprintGlyph, LicenseGlyph } from '@/components/Main/icons'
import type { DocType } from '../../templates'
import type { Doc } from '../DocCell'
import Hologram from '../Hologram'
import type { CardColor } from '../colors'
import IdCard from './IdCard'
import License from './License'
import LicenseBack from './License/Back'
import Page from './Page'
import Permit from './Permit'
import ZoneBack from './ZoneBack'

export interface LayoutProps {
  docType: DocType
  doc: Doc
}

export interface BackProps extends LayoutProps {
  /** The card's mark, large and faint, for the stock of its back. */
  watermark: ReactNode
}

/**
 * What one type of card brings to the shared card (see `Card`): the colour it
 * wears unless the user picked another, its mark — small in the corner of its
 * front, large in the stock of its back — and its other side.
 */
interface CardSpec {
  color: CardColor
  mark: (size: number, doc: Doc) => ReactNode
  Back: ComponentType<BackProps>
}

interface Layout {
  /**
   * What the document measures, in millimetres, as `width / height` — the fixed
   * dimension of a face: the document is this shape, and the printing is
   * arranged to fit inside it.
   */
  format: string
  /** How the document's fields are arranged. */
  Face: ComponentType<LayoutProps>
  /** For a document that is a card. */
  card?: CardSpec
}

// ISO/IEC 7810 and ICAO 9303: a passport's data page is ID-3 and noticeably
// squarer than the ID-1 card a licence, an ID card and a permit all share —
// which is the credit card's own shape, because a credit card is an ID-1 card.
const ID_3 = '125 / 88'
const ID_1 = '85.6 / 53.98'

// A mark drawn in line holds one hairline at either size.
const HAIRLINE = 1.5

/**
 * How each kind of document is drawn. A passport is its data page; a licence,
 * an ID card and a permit are one card (`Card`, with the shared masthead,
 * holder block, foot and cells), each with its own arrangement, default colour
 * and mark. `other` has no physical form to be honest about, so it is a plain
 * page at the wallet-sized default.
 */
export const LAYOUTS: Record<DocType, Layout> = {
  passport: { format: ID_3, Face: Page },
  id_card: {
    format: ID_1,
    Face: IdCard,
    card: {
      color: 'blue',
      // The issuing state's emblem, in film.
      mark: (size, doc) => <Hologram country={doc.value('country')} size={size} />,
      Back: ZoneBack
    }
  },
  driver_license: {
    format: ID_1,
    Face: License,
    card: {
      color: 'rose',
      mark: size => <LicenseGlyph size={size} stroke={HAIRLINE} />,
      Back: LicenseBack
    }
  },
  residence_permit: {
    format: ID_1,
    Face: Permit,
    card: {
      color: 'green',
      mark: size => <FingerprintGlyph size={size} stroke={HAIRLINE} />,
      Back: ZoneBack
    }
  },
  other: { format: ID_1, Face: Page }
}
