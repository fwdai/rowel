import { IdentityGlyph } from '@/components/Main/icons'
import { DOC_GLYPHS } from './glyphs'
import { isDocType } from './templates'

interface Props {
  /** As the row's metadata carries it: free text from the vault. */
  docType?: string
  size: number
}

/**
 * The mark a document wears on a tile — a car for a licence, a house for a
 * permit — and the kind's own for anything that is not a type we know.
 */
export default function DocGlyph({ docType, size }: Props) {
  const Glyph = isDocType(docType) ? DOC_GLYPHS[docType] : IdentityGlyph
  return <Glyph size={size} />
}
