import { useTranslation } from 'react-i18next'
import Button from '@/components/elements/Button'
import { LABEL } from '@/components/elements/tokens'
import { CloudGlyph, DiskGlyph } from '@/components/Main/icons'
import { isMobile } from '@/lib/platform'
import StepHeader from '../shared/StepHeader'
import RestoreAction from '../shared/RestoreAction'
import { COLUMN } from '../shared/layout'

interface Props {
  onFresh: () => void
  onCloud: () => void
  onFile: () => void
}

// The first screen asks one thing: start fresh, or pick up where your own
// data already is. Three ways in and nothing else — a cloud account, behind
// which the providers wait on their own screen (`CloudPicker`) so this one
// does not grow a row per provider, and a backup file. What restoring asks
// for is said on the screen that asks for it. A phone has no file system to
// hand a backup from, so there it is a cloud account or nothing.
export default function Welcome({ onFresh, onCloud, onFile }: Props) {
  const { t } = useTranslation()

  return (
    <>
      <StepHeader
        eyebrow={t('Welcome')}
        title={t('Keep your secrets to yourself.')}
        body={t('Start fresh, or pick up where you left off on another device.')}
      />

      <div className={`${COLUMN} mt-8`}>
        <Button block testid="start-setup-button" onClick={onFresh}>
          {t('Start fresh')}
        </Button>
      </div>

      <div className={`${COLUMN} mt-7 flex items-center gap-3`}>
        <span aria-hidden className="h-px flex-1 bg-line" />
        <span className={LABEL}>{t('Already using Rowel?')}</span>
        <span aria-hidden className="h-px flex-1 bg-line" />
      </div>

      <div className={`${COLUMN} mt-7 flex flex-col gap-2`}>
        <RestoreAction
          mark={<CloudGlyph size={18} />}
          title={t('Continue with a cloud account')}
          body={t('Sync your data to this device.')}
          onClick={onCloud}
          testid="start-cloud-button"
        />
        {!isMobile && (
          <RestoreAction
            mark={<DiskGlyph size={15} />}
            title={t('Restore from a backup file')}
            body={t('A .rowel backup saved on another device.')}
            muted
            onClick={onFile}
            testid="start-restore-button"
          />
        )}
      </div>
    </>
  )
}
