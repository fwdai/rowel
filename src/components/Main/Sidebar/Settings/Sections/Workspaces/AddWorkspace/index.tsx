import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { menuRoom, type MenuRoom } from '@/utils/menuRoom'
import { Dropdown } from '@/components/elements/Dropdown'
import { ChevronDownGlyph, CloudGlyph, DiskGlyph, PlusGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../../sectionNav'
import type { Subpage } from '../../../subpages'
import Way from './Way'

// The one way to one more lockbox, as the list's last row: it opens a menu of
// the three ways there are (the first run's three, offered again beside the
// lockboxes already here), and each leads straight to its own page under
// Lockboxes. A menu rather than a page of its own, so a way is one step from
// the list and Back from it leads to the list.
//
// The card this row closes leaves its overflow open (for the rows' menus, and
// this one), so the row rounds its own bottom corners to match.
export default function AddWorkspace() {
  const { t } = useTranslation()
  const { open: openPage } = useSubpage()
  const [open, setOpen] = useState(false)
  const [side, setSide] = useState<MenuRoom>()
  const trigger = useRef<HTMLButtonElement>(null)

  const toggle = () => {
    if (!open && trigger.current) setSide(menuRoom(trigger.current))
    setOpen(!open)
  }

  const pick = (page: Subpage) => () => {
    setOpen(false)
    openPage(page)
  }

  return (
    <div className="relative">
      <button
        ref={trigger}
        type="button"
        data-testid="workspace-add-row"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={toggle}
        className="flex w-full cursor-pointer items-center gap-3.5 rounded-b-lg px-4 py-3.5 text-left transition-colors hover:bg-hover max-md:rounded-b-xl"
      >
        <span className="grid h-9 w-9 flex-none place-items-center rounded-lg border-[1.5px] border-dashed border-accent-line text-accent">
          <PlusGlyph />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-base font-medium text-accent">{t('Add a lockbox')}</span>
          <span className="mt-0.5 block text-sm text-text2">
            {t('Create a new one, or restore one from a cloud account or a backup')}
          </span>
        </span>
        <ChevronDownGlyph className="flex-none text-text2" />
      </button>
      {open && (
        <Dropdown
          onBlur={() => setOpen(false)}
          // Under the row's own words, and across the row on the phone.
          className={cx(
            'left-3 right-3 md:right-auto md:w-[380px]',
            side?.up ? 'bottom-full -mb-1.5 origin-bottom' : 'top-full -mt-1.5'
          )}
          listMaxHeight={side?.room}
        >
          <Way
            icon={<PlusGlyph />}
            label={t('New lockbox')}
            description={t('A separate encrypted lockbox for work, family or a client')}
            onClick={pick({ key: 'new-workspace' })}
            testid="workspace-new-row"
          />
          {/* A page of its own, so the account's Connect buttons are never read
              as connecting this vault, which is what the same buttons do under
              Sync & backup. */}
          <Way
            icon={<CloudGlyph size={16} />}
            label={t('Restore from a cloud account')}
            description={t('Bring in a lockbox that already exists in your cloud storage')}
            onClick={pick({ key: 'restore-workspace' })}
            testid="workspace-restore-open"
          />
          <Way
            icon={<DiskGlyph size={16} />}
            label={t('Restore a backup')}
            description={t('A backup file and the master password it was sealed with')}
            onClick={pick({ key: 'restore-backup' })}
            testid="workspace-backup-open"
          />
        </Dropdown>
      )}
    </div>
  )
}
