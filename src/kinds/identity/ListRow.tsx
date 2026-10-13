import { IdentityGlyph } from '@/components/Main/icons'
import Row, { type ContentProps } from '@/components/Main/Body/List/Item/Row'
import Cover from './Cover'
import { listSubtitle, passportCountry } from './meta'

// A passport leads with its cover — the issuing country's colour, read off the
// document type and country the row's metadata carries. Every other document,
// and a passport whose country is not a code, wears the kind-tinted glyph.
export default function ListRow({ entry, flag }: ContentProps) {
  const country = passportCountry(entry)

  return (
    <Row
      glyph={country ? <Cover country={country} size={16} /> : <IdentityGlyph size={16} />}
      tint={country ? undefined : 'identity'}
      title={entry.title}
      sub={listSubtitle(entry)}
      flag={flag}
    />
  )
}
