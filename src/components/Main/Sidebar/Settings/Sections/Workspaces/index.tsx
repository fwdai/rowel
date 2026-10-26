import { useTranslation } from 'react-i18next'
import { useApp, selectWorkspaces } from '@/store'
import { cx } from '@/utils/cx'
import SettingsGroup from '@/components/elements/SettingsGroup'
import { CARD_OPEN, LABEL } from '@/components/elements/tokens'
import WorkspaceList from './WorkspaceList'
import NewWorkspace from './NewWorkspace'
import InfoTiles from './InfoTiles'
import RemoteVaults from './RemoteVaults'
import RestoreFromAccount from './RestoreFromAccount'
import RowelServer from './RowelServer'

export default function Workspaces() {
  const { t } = useTranslation()
  const count = useApp(selectWorkspaces).length

  return (
    <>
      {/* A SettingsGroup, spelled out: the info tiles belong to the group,
          under its card. The card's overflow stays open so a row's menu can
          hang past the last row. */}
      <section className="mb-7">
        <div className={cx(LABEL, 'mb-2')}>
          {t('On this device')} · {count}
        </div>
        <div className={CARD_OPEN}>
          <WorkspaceList />
          <NewWorkspace />
        </div>
        <InfoTiles />
      </section>
      <RemoteVaults />
      <SettingsGroup label={t('Cloud sync')}>
        <RestoreFromAccount />
      </SettingsGroup>
      <RowelServer />
    </>
  )
}
