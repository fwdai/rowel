import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { ImportFormat } from '@/api/imports'
import { Dropdown, DropdownItem, DropdownMeta } from '@/components/elements/Dropdown'
import { MoreGlyph } from '../../../../icons'
import { OTHERS } from './apps'
import Tile from './Tile'

interface Props {
  active: string | null
  disabled: boolean
  onFormat: (format: ImportFormat) => void
}

// The sixth tile: the apps that get no tile of their own, behind a menu. It
// looks like the others and lights the same way when one of its apps is the
// source; picking an item opens the file picker just as a tile does, and
// picking the same one again picks another file.
export default function OtherTile({ active, disabled, onFormat }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const current = OTHERS.find(app => app.key === active)

  const pick = (format: ImportFormat) => {
    setOpen(false)
    onFormat(format)
  }

  return (
    <div className="relative">
      <Tile
        badge={<MoreGlyph size={16} />}
        name={current ? t(current.name) : t('Another app')}
        hint={current ? t(current.hint) : t('Chrome / Safari, CXF or CSV')}
        active={current !== undefined}
        disabled={disabled}
        onClick={() => setOpen(!open)}
        testid="import-tile-other"
        haspopup="menu"
        expanded={open}
      />
      {open && (
        <Dropdown onBlur={() => setOpen(false)} className="left-0 top-[66px] w-full min-w-[220px]">
          {OTHERS.map(app => (
            <DropdownItem
              key={app.key}
              testid={`import-tile-${app.key}`}
              onClick={() => pick(app.format)}
            >
              <span className="min-w-0 flex-1 truncate">{t(app.name)}</span>
              <DropdownMeta>{t(app.hint)}</DropdownMeta>
            </DropdownItem>
          ))}
        </Dropdown>
      )}
    </div>
  )
}
