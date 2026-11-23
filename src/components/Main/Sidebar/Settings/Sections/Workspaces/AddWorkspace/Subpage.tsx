import { useTranslation } from 'react-i18next'
import SettingsNavRow from '@/components/elements/SettingsNavRow'
import { CARD } from '@/components/elements/tokens'
import { CloudGlyph, DiskGlyph, PlusGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../../sectionNav'
import SubpageScroller from '../../../SubpageScroller'

// Lockboxes › Add a lockbox: the three ways to one more, each a page of its
// own opened from here. They are the first run's three, offered again beside
// the lockboxes already on this device.
export default function AddWorkspaceSubpage() {
  const { t } = useTranslation()
  const { open } = useSubpage()

  return (
    <SubpageScroller testid="settings-add-workspace">
      {/* No heading over the card: the page's own title is the heading. */}
      <div className={CARD}>
        <SettingsNavRow
          icon={<PlusGlyph />}
          label={t('New lockbox')}
          description={t('A separate encrypted lockbox for work, family or a client')}
          onClick={() => open({ key: 'new-workspace' })}
          testid="workspace-new-row"
        />
        {/* Its own page, so the account's Connect buttons are never read as
            connecting this vault, which is what the same buttons do under
            Sync & backup. */}
        <SettingsNavRow
          icon={<CloudGlyph size={16} />}
          label={t('Restore from a cloud account')}
          description={t('Bring in a lockbox that already exists in your cloud storage')}
          onClick={() => open({ key: 'restore-workspace' })}
          testid="workspace-restore-open"
        />
        <SettingsNavRow
          icon={<DiskGlyph size={16} />}
          label={t('Restore a backup')}
          description={t('A backup file and the master password it was sealed with')}
          onClick={() => open({ key: 'restore-backup' })}
          testid="workspace-backup-open"
        />
      </div>
    </SubpageScroller>
  )
}
