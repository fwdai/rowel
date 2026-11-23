import { useTranslation } from 'react-i18next'
import { useApp, selectWorkspaces } from '@/store'
import { cx } from '@/utils/cx'
import { CARD_OPEN, LABEL } from '@/components/elements/tokens'
import { PlusGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../sectionNav'
import WorkspaceList from './WorkspaceList'
import AddRow from './AddRow'
import RemoteVaults from './RemoteVaults'
import RowelServer from './RowelServer'

export default function Workspaces() {
  const { t } = useTranslation()
  const count = useApp(selectWorkspaces).length
  const { open } = useSubpage()

  return (
    <>
      {/* A SettingsGroup, spelled out: the card's overflow stays open so a
          row's menu can hang past the last row. */}
      <section className="mb-7">
        <div className={cx(LABEL, 'mb-2')}>
          {t('On this device')} · {count}
        </div>
        <div className={CARD_OPEN}>
          <WorkspaceList />
          {/* The one way to one more lockbox; which kind is asked on its page. */}
          <AddRow
            icon={<PlusGlyph />}
            label={t('Add a lockbox')}
            description={t('Create a new one, or restore one from a cloud account or a backup')}
            onClick={() => open({ key: 'add-workspace' })}
            testid="workspace-add-row"
          />
        </div>
      </section>
      <RemoteVaults />
      <RowelServer />
    </>
  )
}
