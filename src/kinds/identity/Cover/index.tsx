import genericEmblem from '@/assets/images/passport-emblem.svg'
import { useEmblem } from '@/hooks/useEmblem'
import { coverColor } from './colors'
import Foil from './Foil'

interface Props {
  /** ISO 3166-1 alpha-3, as `passportCountry` (see `../meta`) hands it out. */
  country: string
}

/**
 * A passport as its cover shows it: the issuing country's colour, with its coat
 * of arms stamped on it in foil — fetched once per country from the website
 * and kept in the vault (see `src-tauri/src/emblem.rs`). Until it arrives, or
 * when the country has none, a generic one stands in: a globe in a laurel
 * wreath, in the same foil, so every passport wears an emblem. Fills the tile
 * it is set in, the way a favicon does.
 */
export default function Cover({ country }: Props) {
  const emblem = useEmblem(country)

  return (
    <div
      data-testid="passport-cover"
      data-country={country}
      data-emblem={emblem ? 'country' : 'generic'}
      className="grid h-full w-full place-items-center"
      style={{ backgroundColor: coverColor(country) }}
    >
      <Foil src={emblem ?? genericEmblem} className="h-[68%] w-[68%]" />
    </div>
  )
}
