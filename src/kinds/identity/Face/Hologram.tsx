import genericEmblem from '@/assets/images/passport-emblem.svg'
import { useEmblem } from '@/hooks/useEmblem'
import { countryCode } from '@/utils/countries'
import Foil from '../Cover/Foil'

// A hologram's colours as the card is tilted through them, all at once.
const IRIDESCENT =
  'linear-gradient(125deg, #5CC6DE 0%, #8E9BF2 30%, #D790D8 55%, #F2A7B4 75%, #F0CF86 100%)'

interface Props {
  /** The issuing country as the document has it; anything not a code gets the generic emblem. */
  country: string
  className?: string
}

/**
 * The issuing state's coat of arms as the hologram a card carries over its
 * photograph: the same emblem a passport's cover is stamped with (see `Cover`),
 * in the shifting film instead of gold.
 */
export default function Hologram({ country, className }: Props) {
  const emblem = useEmblem(countryCode(country))

  return (
    <Foil
      src={emblem ?? genericEmblem}
      metal={IRIDESCENT}
      testid="identity-hologram"
      className={className}
    />
  )
}
