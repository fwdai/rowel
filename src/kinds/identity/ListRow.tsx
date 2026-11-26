import Row, { type ContentProps } from '@/components/Main/Body/List/Item/Row'
import Cover from './Cover'
import DocGlyph from './DocGlyph'
import { tileOf } from './Face/colors'
import { cardColorFor } from './Face/layouts'
import { listSubtitle, passportCountry } from './meta'

// A passport leads with its cover — the issuing country's colour, read off the
// document type and country the row's metadata carries. A card leads with its
// own glyph (a car for a licence, a house for a permit) on a tile in the card's
// colour — the one picked for it, or its type's — so the list can be scanned by
// the colour of the card being looked for. Anything else wears the kind's tint.
export default function ListRow({ entry, flag }: ContentProps) {
  const country = passportCountry(entry)
  const color = cardColorFor(entry.docType, entry.docColor)
  const glyph = <DocGlyph docType={entry.docType} size={16} />

  return (
    <Row
      glyph={
        country ? (
          <Cover country={country} />
        ) : color ? (
          <div
            data-testid="card-tile"
            data-color={color}
            className="grid h-full w-full place-items-center"
            style={tileOf(color)}
          >
            {glyph}
          </div>
        ) : (
          glyph
        )
      }
      tint={country || color ? undefined : 'identity'}
      title={entry.title}
      sub={listSubtitle(entry)}
      flag={flag}
    />
  )
}
