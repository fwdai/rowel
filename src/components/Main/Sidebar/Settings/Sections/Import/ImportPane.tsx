import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { LABEL } from '@/components/elements/tokens'
import type { useImport } from './useImport'
import { useExchangeSupported } from './useExchangeSupported'
import DropZone from './DropZone'
import Tiles from './Tiles'
import BackupCard from './BackupCard'
import Result from './Result'

// Bringing secrets in: any export file up top, then the apps we know by name
// (the popular ones as tiles, the rest behind the last tile's menu), then a
// backup of our own.
export default function ImportPane({ flow }: { flow: ReturnType<typeof useImport> }) {
  const { t } = useTranslation()
  const exchange = useExchangeSupported()
  // Which tile lit: a `.rowel` and another app's data have none, they came
  // from the OS, not a tile.
  const { picked } = flow
  const active =
    picked === null || picked.kind === 'rowel' || picked.kind === 'exchange'
      ? null
      : picked.kind === 'swftx'
        ? 'swftx'
        : picked.kind === 'exchangeGuide'
          ? 'apple'
          : picked.format

  return (
    <>
      <DropZone onDrop={flow.dropped} disabled={flow.running} />
      {/* The same rhythm as every other group: its label 8px over its cards. */}
      <div className={cx(LABEL, 'mt-7 mb-2')}>{t('Or start from an app')}</div>
      <Tiles
        active={active}
        disabled={flow.running}
        onFormat={flow.chooseFile}
        exchange={exchange}
        onExchange={flow.explainExchange}
      />
      <div className="mt-3">
        <BackupCard
          active={active === 'swftx'}
          disabled={flow.running}
          onBackup={flow.chooseBackup}
        />
      </div>
      <Result flow={flow} />
      <p className="mt-3 text-sm text-text2">
        {t('Imports are merged. Existing items are never overwritten.')}
      </p>
    </>
  )
}
