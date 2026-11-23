import { useTranslation } from 'react-i18next'
import type { RemotePack } from '@/api/setup'
import { useDates } from '@/hooks/useDates'
import { describeRemotePack } from '@/utils/remotePack'
import SettingsNavRow from '@/components/elements/SettingsNavRow'
import { CloudGlyph } from '@/components/Main/icons'
import { useSectionNav } from '../../../../sectionNav'
import Standing from './Standing'

// A lockbox the account holds and this device does not. A pack carries no name
// readable before it is unsealed, so it is told apart by when it was written
// and its size. Restoring it takes its master password, and the form for that
// is Settings › Lockboxes', beside every other way a lockbox arrives here, so
// the row leads there rather than repeating it.
export default function RemoteRow({ pack }: { pack: RemotePack }) {
  const { t } = useTranslation()
  const go = useSectionNav()
  const dates = useDates()
  const testid = `settings-cloud-remote-${pack.id}`

  return (
    <SettingsNavRow
      icon={<CloudGlyph size={16} />}
      label={t('Rowel lockbox')}
      description={
        <Standing
          standing="account"
          provider={pack.provider}
          detail={describeRemotePack(pack, dates)}
          testid={`${testid}-status`}
        />
      }
      onClick={() => go('workspaces')}
      testid={testid}
    />
  )
}
