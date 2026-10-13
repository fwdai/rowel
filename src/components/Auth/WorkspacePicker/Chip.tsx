import type { Ref } from 'react'
import { useTranslation } from 'react-i18next'
import type { Workspace } from '@/api/types'
import { workspaceLabel } from '@/lib/workspace'
import { cx } from '@/utils/cx'
import Monogram from '@/components/elements/Monogram'
import { ChevronDownGlyph } from '@/components/Main/icons'

interface Props {
  workspace: Workspace
  open: boolean
  onClick: () => void
  ref: Ref<HTMLButtonElement>
}

// The vault about to open: its tile, its name and a chevron that
// turns over while the menu is down. Named by the vault, so assistive
// technology hears which one rather than a generic label.
export default function Chip({ workspace, open, onClick, ref }: Props) {
  const { t } = useTranslation()
  const label = workspaceLabel(workspace, t)

  return (
    <button
      ref={ref}
      type="button"
      data-testid="workspace-chip"
      aria-haspopup="menu"
      aria-expanded={open}
      title={t('Switch lockbox')}
      onClick={onClick}
      className={cx(
        // Concentric with the tile it holds: the tile's 7px corner (30% of 24)
        // plus the 5px inset is 12 — one step under the menu's 14, so the chip
        // and the panel it drops read as the same family of shapes.
        'relative flex h-[34px] cursor-pointer items-center gap-2 rounded-lg pl-[5px] pr-2.5 text-[14px] font-medium tracking-[-0.01em] text-text transition-[background-color,box-shadow]',
        // A finger gets the phone's 44px target, reaching past the drawn chip
        // rather than growing it: the chip looks the same on every device.
        "any-pointer-coarse:before:absolute any-pointer-coarse:before:inset-x-0 any-pointer-coarse:before:-inset-y-[5px] any-pointer-coarse:before:content-['']",
        open ? 'bg-tile shadow-[inset_0_0_0_0.5px_var(--c-line2)]' : 'bg-hover hover:bg-tile'
      )}
    >
      <Monogram name={label} seed={workspace.id} color={workspace.color} size={24} />
      <span className="max-w-[180px] truncate">{label}</span>
      <span
        className={cx(
          'grid place-items-center text-text3 transition-transform duration-200',
          open && 'rotate-180'
        )}
      >
        <ChevronDownGlyph size={10} stroke={1.5} />
      </span>
    </button>
  )
}
