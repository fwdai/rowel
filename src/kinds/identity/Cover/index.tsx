import { IdentityGlyph } from '@/components/Main/icons'
import { coverColor } from './colors'

// The foil a cover's printing is stamped in. Fixed, like the cover under it:
// a passport is the same object in the dark theme.
const FOIL = '#D9B76A'

interface Props {
  /** ISO 3166-1 alpha-3, as `passportCountry` (see `../meta`) hands it out. */
  country: string
  /** The mark stamped on it, in px. */
  size: number
}

/**
 * A passport as its cover shows it: the issuing country's colour, with the mark
 * stamped on it in foil. Fills the tile it is set in, the way a favicon does.
 */
export default function Cover({ country, size }: Props) {
  return (
    <div
      data-testid="passport-cover"
      data-country={country}
      className="grid h-full w-full place-items-center"
      style={{ backgroundColor: coverColor(country), color: FOIL }}
    >
      <IdentityGlyph size={size} />
    </div>
  )
}
