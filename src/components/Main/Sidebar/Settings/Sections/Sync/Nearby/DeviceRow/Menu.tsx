import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import IconButton from '@/components/elements/IconButton'
import { Dropdown, DropdownItem } from '@/components/elements/Dropdown'
import { MoreGlyph, TrashGlyph } from '@/components/Main/icons'

interface Props {
  id: string
  paused: boolean
  onPause: (paused: boolean) => void
  onRemove: () => void
}

// A device's actions, behind the row's ⋯: there is nothing to do on a row
// that is syncing as it should.
export default function Menu({ id, paused, onPause, onRemove }: Props) {
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
        testid={`settings-nearby-menu-${id}`}
        onClick={() => setOpen(value => !value)}
        className="any-pointer-coarse:h-11 any-pointer-coarse:w-11"
      >
        <MoreGlyph />
      </IconButton>
      {open && (
        <Dropdown onBlur={() => setOpen(false)} className="right-0 top-full mt-2 w-[220px]">
          <DropdownItem
            testid={`settings-nearby-pause-${id}`}
            onClick={pick(() => onPause(!paused))}
          >
            {paused ? t('Resume syncing') : t('Pause syncing')}
          </DropdownItem>
          <DropdownItem
            separated
            danger
            testid={`settings-nearby-remove-${id}`}
            onClick={pick(onRemove)}
          >
            <TrashGlyph />
            {t('Remove…')}
          </DropdownItem>
        </Dropdown>
      )}
    </div>
  )
}
