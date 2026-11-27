import { useTranslation } from 'react-i18next'
import { useSubpage } from '../../sectionNav'
import ActionNote from './ActionNote'

// One of our own backups, picked on the backup card, dropped or opened by the
// OS while a vault is open: a whole sealed database, not rows to merge. It
// restores as a lockbox of its own, on the page for that, opened over this
// one and handed the path: the pick, the drop or the open granted it, and
// nothing has read it since, so it is not picked again there.
export default function RowelNotice({ path }: { path: string }) {
  const { t } = useTranslation()
  const { open } = useSubpage()

  return (
    <ActionNote
      testid="import-rowel-notice"
      action={t('Restore it as a lockbox')}
      actionTestid="import-rowel-restore"
      onAction={() => open({ key: 'restore-backup', path })}
    >
      {t('This is a {{appName}} backup. It is restored as a lockbox of its own, not imported.')}
    </ActionNote>
  )
}
