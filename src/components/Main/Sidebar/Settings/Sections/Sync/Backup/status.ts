import { useTranslation } from 'react-i18next'
import { selectWorkspaces, useApp } from '@/store'
import { useDates } from '@/hooks/useDates'
import { backupCoverage } from '@/lib/workspace'

// The Back up the vault row's status on the overview: where backups stand
// across every lockbox that can have one. Until each has been saved, how many
// have; once each has, how far back the oldest goes, since that is how old
// the set of them is — said as the lockbox's own last save when there is one.
export const useBackupStatus = (): string => {
  const { t } = useTranslation()
  const { relativeLong } = useDates()
  const { lockboxes, saved, oldest } = backupCoverage(useApp(selectWorkspaces))

  if (oldest === null) return saved === 0 ? t('Never saved') : t('Some lockboxes never saved')
  const when = relativeLong(oldest)
  return lockboxes === 1
    ? t('Last saved {{when}}', { when })
    : t('Oldest backup {{when}}', { when })
}
