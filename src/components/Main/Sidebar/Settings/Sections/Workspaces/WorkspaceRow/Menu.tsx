import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import IconButton from '@/components/elements/IconButton'
import { Dropdown, DropdownItem } from '@/components/elements/Dropdown'
import { META } from '@/components/elements/tokens'
import { MoreGlyph, PencilGlyph, TrashGlyph } from '@/components/Main/icons'

interface Props {
  id: string
  // The device's only workspace: it stays, so Delete is shown but inert.
  last: boolean
  onEdit: () => void
  onDelete: () => void
}

// A workspace's less frequent actions, behind the row's ⋯.
export default function Menu({ id, last, onEdit, onDelete }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)

  const pick = (action: () => void) => () => {
    setOpen(false)
    action()
  }

  return (
    <div className="relative">
      <IconButton
        label={t('More actions')}
        expanded={open}
        testid={`workspace-menu-${id}`}
        onClick={() => setOpen(value => !value)}
        className="any-pointer-coarse:h-11 any-pointer-coarse:w-11"
      >
        <MoreGlyph />
      </IconButton>
      {open && (
        <Dropdown onBlur={() => setOpen(false)} className="right-0 top-full mt-2 w-[220px]">
          <DropdownItem testid={`workspace-edit-${id}`} onClick={pick(onEdit)}>
            <PencilGlyph />
            {t('Edit…')}
          </DropdownItem>
          {/* A disabled fieldset is what makes the item a disabled button
              without the menu item having to know about it. */}
          <fieldset disabled={last} className="contents">
            <DropdownItem
              separated
              danger
              testid={`workspace-delete-${id}`}
              onClick={pick(onDelete)}
              className="disabled:cursor-default disabled:opacity-50 disabled:hover:bg-transparent"
            >
              <TrashGlyph />
              {t('Delete')}
            </DropdownItem>
          </fieldset>
          {last && (
            <p className={`${META} px-2 pb-1 pt-0.5`}>
              {t('Your only workspace cannot be deleted. Add another one first.')}
            </p>
          )}
        </Dropdown>
      )}
    </div>
  )
}
