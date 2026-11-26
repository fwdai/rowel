import Row, { type ContentProps } from '@/components/Main/Body/List/Item/Row'
import Cover from './Cover'
import DocGlyph from './DocGlyph'
import { listSubtitle, passportCountry } from './meta'

// A passport leads with its cover — the issuing country's colour, read off the
// document type and country the row's metadata carries. Every other document,
// and a passport whose country is not a code, wears its own glyph on the kind's
// tint: a car for a licence, a house for a permit.
export default function ListRow({ entry, flag }: ContentProps) {
  const country = passportCountry(entry)

  return (
    <Row
      glyph={country ? <Cover country={country} /> : <DocGlyph docType={entry.docType} size={16} />}
      tint={country ? undefined : 'identity'}
      title={entry.title}
      sub={listSubtitle(entry)}
      flag={flag}
    />
  )
}
