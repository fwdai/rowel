import { useTranslation } from 'react-i18next'
import Brand from './Brand'
import Add from './Add'
import ViewButton from './ViewButton'
import Tags from './Tags'
import Lock from './Lock'
import Settings from './Settings'
import { ArchiveRailGlyph, GridRailGlyph, StarRailGlyph } from '../icons'

// The 56px icon rail, in three groups: brand mark · new-secret (the one verb)
// at the head; all-items · favorites · archive · tags (the places) in the
// middle; lock · settings (the vault itself) at the foot. Rail tiles are 36px
// with 20px glyphs — one step up from the in-pane tiers so the rail reads as
// primary navigation.
// Tags is a view like the three above it, not a filter over them: its menu
// lists the vault's tags, and a picked tag gathers its items from across the
// vault under the Tags tile.
// The generator is a tool, not a place, so it has no tile: it opens from ⌘G
// and the command palette, or from the password field of the login it is for.
// The Vault Health tile is parked, not removed: `VaultHealth.tsx` and the
// `health` view stay, reachable from Settings › Audit.
export default function Sidebar() {
  const { t } = useTranslation()
  return (
    <nav className="flex w-[56px] flex-none flex-col items-center gap-1.5 border-r border-line bg-rail py-3">
      <Brand />
      <div className="my-2 h-px w-5 bg-line2" />
      <Add />
      <div className="h-1.5" />
      <ViewButton view="items" label={t('All Items')} testid="view-items">
        <GridRailGlyph />
      </ViewButton>
      <ViewButton view="favorites" label={t('Favorites')} testid="view-favorites">
        <StarRailGlyph />
      </ViewButton>
      <ViewButton view="archive" label={t('Archive')} testid="view-archive">
        <ArchiveRailGlyph />
      </ViewButton>
      <Tags />
      <div className="flex-1" />
      <Lock />
      <Settings />
    </nav>
  )
}
