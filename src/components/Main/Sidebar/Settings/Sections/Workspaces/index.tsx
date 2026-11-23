import { useTranslation } from 'react-i18next'
import { useApp, selectWorkspaces } from '@/store'
import { cx } from '@/utils/cx'
import { CARD_OPEN, LABEL } from '@/components/elements/tokens'
import WorkspaceList from './WorkspaceList'
import AddWorkspace from './AddWorkspace'
import RemoteVaults from './RemoteVaults'
import RowelServer from './RowelServer'

export default function Workspaces() {
  const { t } = useTranslation()
  const count = useApp(selectWorkspaces).length

  return (
    <>
      {/* A SettingsGroup, spelled out: the card's overflow stays open so a
          row's menu, and the Add row's, can hang past the last row. */}
      <section className="mb-7">
        <div className={cx(LABEL, 'mb-2')}>
          {t('On this device')} · {count}
        </div>
        <div className={CARD_OPEN}>
          <WorkspaceList />
          <AddWorkspace />
        </div>
      </section>
      <RemoteVaults />
      <RowelServer />
    </>
  )
}
