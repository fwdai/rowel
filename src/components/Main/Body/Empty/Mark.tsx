import Logo from '@/assets/images/logo.svg?react'
import { CENTER, pointOnSpike } from '@/components/elements/asteriskGeometry'
import type { Glyph } from '@/kinds'

interface Props {
  size?: number
  // A view's own symbol (the star, the archive box), worn as a badge.
  badge?: Glyph
}

// The badge sits on the lower-right spike's head, cutting into the mark the
// way a system badge cuts into an app icon, so a view's symbol reads as
// something the mark holds rather than a sticker on top of it. Read off the
// geometry, so it follows the spike if the mark is ever re-baked.
const head = pointOnSpike(120, 24)
const BADGE_AT = {
  left: `${(head.x / (CENTER * 2)) * 100}%`,
  top: `${(head.y / (CENTER * 2)) * 100}%`
}

// The brand mark as an empty state wears it: the rail's own baked logo, full
// brand ink, at hero size. The same character the rail shows at 24px and the
// lock screen shows with a face, so an empty pane still reads as the app.
export default function Mark({ size = 64, badge: Badge }: Props) {
  return (
    <div className="relative flex-none text-brand" style={{ width: size, height: size }}>
      <Logo width={size} height={size} className="fill-current" aria-hidden="true" />
      {Badge && (
        <span
          aria-hidden="true"
          style={BADGE_AT}
          // Ringed in the pane's own ground so the badge cuts into the mark
          // the way a system badge cuts into an app icon.
          className="absolute grid h-[26px] w-[26px] -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full bg-accent text-accent-fg ring-[3px] ring-detail max-md:ring-screen"
        >
          <Badge size={13} />
        </span>
      )}
    </div>
  )
}
