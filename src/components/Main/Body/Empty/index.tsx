import type { ReactElement, ReactNode } from 'react'
import { useUi } from '@/store'
import { cx } from '@/utils/cx'
import { PANE_BLEED } from '@/components/elements/tokens'
import { useVariant, type Variant } from './variant'
import {
  KindEmpty,
  SearchEmpty,
  SelectEmpty,
  VaultEmpty,
  HealthEmpty,
  FavoritesEmpty,
  ArchiveEmpty
} from './variants'
import Watermark from './Watermark'

// The list column's share: only the filter states, and only ever as one line.
// On `vault` and `health` it renders nothing at all — the detail pane is
// saying it, and a second empty state next to it would just be noise.
export default function ListEmpty() {
  const variant = useVariant()
  const type = useUi(state => state.filterType)
  const query = useUi(state => state.query)

  if (variant === 'search') return <SearchEmpty query={query.trim()} type={type} />
  // `kind` is unreachable without a filter, but the type still has to narrow.
  if (variant === 'kind' && type) return <KindEmpty type={type} />
  return null
}

// The filter states have already been explained in the list column, so the
// pane stays on the quiet select rather than answering them a second time.
const DETAIL: Record<Variant, () => ReactElement> = {
  vault: VaultEmpty,
  health: HealthEmpty,
  favorites: FavoritesEmpty,
  archive: ArchiveEmpty,
  kind: SelectEmpty,
  search: SelectEmpty,
  select: SelectEmpty
}

// The detail pane's share: every variant lands somewhere, centered in the pane.
export function DetailEmpty({ variant }: { variant: Variant }) {
  const Content = DETAIL[variant]

  return (
    <PaneSurface>
      <Content />
    </PaneSurface>
  )
}

// The ground under it, and under anything else the pane shows in place of an
// entry (a batch's summary). On the wide shell the surface takes the whole
// pane back from its inset and clips, so the watermark can run off the
// corner; the content keeps its own gutter so it never meets an edge in a
// narrow split. It grows to fill the
// pane but never shrinks under its content (`min-h-auto`, the flex item's
// content-based minimum): in a short window the surface gets taller than the
// pane and the pane scrolls, rather than the clip taking the sheet with it.
// `overflow-clip`, not `hidden`: hidden makes the surface a scroll container,
// and a flex item that scrolls has an automatic minimum of zero — the
// `min-h-auto` above would be void and the sheet cut off at both ends. Clip
// only trims the watermark at the edge and leaves the sizing alone.
export function PaneSurface({ children }: { children: ReactNode }) {
  return (
    <div
      className={cx(
        'relative flex min-h-full flex-col items-center justify-center px-8 py-10 md:min-h-auto md:flex-1 md:overflow-clip',
        PANE_BLEED
      )}
    >
      <Watermark />
      {children}
    </div>
  )
}
