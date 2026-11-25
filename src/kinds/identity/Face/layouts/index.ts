import type { ComponentType } from 'react'
import type { DocType } from '../../templates'
import type { Doc } from '../DocCell'
import IdCard from './IdCard'
import License from './License'
import LicenseBack from './License/Back'
import Page from './Page'
import Permit from './Permit'
import ZoneBack from './ZoneBack'
import { ID_CARD, LICENSE, PERMIT, type Stock } from './stocks'

export interface LayoutProps {
  docType: DocType
  doc: Doc
}

export interface BackProps extends LayoutProps {
  stock: Stock
}

interface Layout {
  /**
   * What the document measures, in millimetres, as `width / height` — the fixed
   * dimension of a face: the document is this shape, and the printing is
   * arranged to fit inside it.
   */
  format: string
  Face: ComponentType<LayoutProps>
  /** For a document that is a card: what it is made of, and its other side. */
  card?: { stock: Stock; Back: ComponentType<BackProps> }
}

// ISO/IEC 7810 and ICAO 9303: a passport's data page is ID-3 and noticeably
// squarer than the ID-1 card a licence, an ID card and a permit all share —
// which is the credit card's own shape, because a credit card is an ID-1 card.
const ID_3 = '125 / 88'
const ID_1 = '85.6 / 53.98'

/**
 * How each kind of document is drawn. A passport is its data page; a licence,
 * an ID card and a permit are each the card they are, front and back. `other`
 * has no physical form to be honest about, so it is a plain page at the
 * wallet-sized default.
 */
export const LAYOUTS: Record<DocType, Layout> = {
  passport: { format: ID_3, Face: Page },
  id_card: { format: ID_1, Face: IdCard, card: { stock: ID_CARD, Back: ZoneBack } },
  driver_license: { format: ID_1, Face: License, card: { stock: LICENSE, Back: LicenseBack } },
  residence_permit: { format: ID_1, Face: Permit, card: { stock: PERMIT, Back: ZoneBack } },
  other: { format: ID_1, Face: Page }
}
