import type { ComponentType } from 'react'
import type { DocType } from '../../templates'
import type { Doc } from '../DocCell'
import IdCard from './IdCard'
import IdCardBack from './IdCard/Back'
import License from './License'
import LicenseBack from './License/Back'
import Page from './Page'
import Permit from './Permit'
import PermitBack from './Permit/Back'

export interface LayoutProps {
  docType: DocType
  doc: Doc
  /** Turns the card over; absent on a document that has no back to turn to. */
  flip?: () => void
}

/** What a card's stock is printed with, the same on both its sides (see `Print`). */
export interface Stock {
  wash: string
  line: string
  engraving: string
}

interface Layout {
  /**
   * What the document measures, in millimetres, as `width / height` — the fixed
   * dimension of a face: the document is this shape, and the printing is
   * arranged to fit inside it.
   */
  format: string
  Face: ComponentType<LayoutProps>
  /** The other side, for a document that is a card. */
  Back?: ComponentType<LayoutProps>
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
  id_card: { format: ID_1, Face: IdCard, Back: IdCardBack },
  driver_license: { format: ID_1, Face: License, Back: LicenseBack },
  residence_permit: { format: ID_1, Face: Permit, Back: PermitBack },
  other: { format: ID_1, Face: Page }
}
