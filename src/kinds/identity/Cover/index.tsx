import { IdentityGlyph } from '@/components/Main/icons'
import { useEmblem } from '@/hooks/useEmblem'
import { coverColor } from './colors'

// The foil a cover's printing is stamped in. Fixed, like the cover under it:
// a passport is the same object in the dark theme.
const FOIL = '#D9B76A'

interface Props {
  /** ISO 3166-1 alpha-3, as `passportCountry` (see `../meta`) hands it out. */
  country: string
  /** The foil mark's size, in px, while there is no emblem to show. */
  size: number
}

/**
 * A passport as its cover shows it: the issuing country's colour, with its coat
 * of arms on it — fetched once per country and kept in the vault (see
 * `src-tauri/src/emblem`). Until it arrives, or when the country has none, the
 * identity mark stands in, stamped in foil. Fills the tile it is set in, the
 * way a favicon does.
 */
export default function Cover({ country, size }: Props) {
  const emblem = useEmblem(country)

  return (
    <div
      data-testid="passport-cover"
      data-country={country}
      className="grid h-full w-full place-items-center"
      style={{ backgroundColor: coverColor(country), color: FOIL }}
    >
      {emblem ? (
        <img src={emblem} alt="" className="h-[68%] w-[68%] object-contain" />
      ) : (
        <IdentityGlyph size={size} />
      )}
    </div>
  )
}
