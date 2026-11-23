import { useTranslation } from 'react-i18next'
import Note from '@/components/elements/Note'
import { TEXT_LINK } from '@/components/elements/tokens'
import { useSectionNav } from '../../sectionNav'

// One of our own backups, picked on the backup card, dropped or opened by the
// OS while a vault is open: a whole sealed database, not rows to merge. It
// restores as a lockbox of its own, on the Lockboxes page for that, which is
// handed the path: the pick, the drop or the open granted it, and nothing has
// read it since, so it is not picked again there.
export default function RowelNotice({ path }: { path: string }) {
  const { t } = useTranslation()
  const go = useSectionNav()

  return (
    <Note testid="import-rowel-notice">
      {t('This is a {{appName}} backup. It is restored as a lockbox of its own, not imported.')}{' '}
      <button
        type="button"
        data-testid="import-rowel-restore"
        onClick={() => go('workspaces', { key: 'restore-backup', path })}
        className={TEXT_LINK}
      >
        {t('Restore it as a lockbox')}
      </button>
    </Note>
  )
}
