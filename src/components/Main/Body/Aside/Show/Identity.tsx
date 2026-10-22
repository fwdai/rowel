import type { ReactNode } from 'react'
import type { EntryMeta } from '@/api/types'
import { cx } from '@/utils/cx'
import CardBrandMark from '@/components/elements/CardBrandMark'
import { hasBrandMark } from '@/utils/cardBrand'
import { kindOf } from '@/kinds'
import Cover from '@/kinds/identity/Cover'
import { passportCountry } from '@/kinds/identity/meta'
import { useFavicon } from '@/hooks/useFavicon'
import { TITLE, type Look } from './look'

interface TileProps {
  look: Look
  /** The tile's box — 28px in the desktop header, 44px on the phone screen. */
  tile: string
  /** The mark inside it, in px. Scales with the tile; nothing else does. */
  glyph: number
}

// The entry's own artwork if it has any — a favicon, a card brand mark, a
// passport's cover — and the kind's glyph otherwise, on the tile both headers
// put beside the title, reading and writing alike.
export function Tile({ look, tile, glyph }: TileProps) {
  const icon = useFavicon(look.urlHost)
  const country = passportCountry(look)
  const Glyph = kindOf(look.type).Glyph
  // Brand marks are wider than they are tall, so they ride 4px under the kind
  // glyph's edge at every size — the pairing the header has always drawn.
  const mark = glyph - 4

  return (
    <div
      className={cx(
        'grid',
        tile,
        'flex-none place-items-center overflow-hidden rounded-lg bg-tile text-text2 max-md:rounded-xl'
      )}
    >
      {icon ? (
        <img src={icon} alt="" className="h-full w-full object-cover" />
      ) : hasBrandMark(look.cardBrand) ? (
        <CardBrandMark brand={look.cardBrand} size={mark} />
      ) : country ? (
        <Cover country={country} />
      ) : (
        <Glyph size={glyph} />
      )}
    </div>
  )
}

interface Props {
  entry: EntryMeta
  tile: string
  glyph: number
  /** The row the tile and the title sit on. */
  className?: string
  /** An eyebrow to share the title's column with (the phone puts it here). */
  children?: ReactNode
}

// Who the entry is: its artwork on the tile, beside the title. One component,
// two sizes: both shells draw the same identity, only bigger where there is room.
export default function Identity({ entry, tile, glyph, className, children }: Props) {
  const title = <h1 className={`truncate border-transparent ${TITLE}`}>{entry.title}</h1>

  return (
    <div className={className}>
      <Tile look={entry} tile={tile} glyph={glyph} />
      {/* Given an eyebrow, the title stops being the row's only text and the
          two share a column, the name first and its address under it (the
          phone header's "aws.amazon.com · Work"); without one it is the row
          itself. */}
      {children ? (
        <div className="min-w-0 flex-1">
          {title}
          {children}
        </div>
      ) : (
        title
      )}
    </div>
  )
}
