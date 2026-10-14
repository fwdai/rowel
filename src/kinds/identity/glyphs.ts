import {
  DocumentGlyph,
  IdentityGlyph,
  LicenseGlyph,
  PassportGlyph,
  ResidenceGlyph
} from '@/components/Main/icons'
import type { Glyph } from '../types'
import type { DocType } from './templates'

/** The mark each document wears: on the add picker's list, over the form. */
export const DOC_GLYPHS: Record<DocType, Glyph> = {
  passport: PassportGlyph,
  id_card: IdentityGlyph,
  driver_license: LicenseGlyph,
  residence_permit: ResidenceGlyph,
  other: DocumentGlyph
}
