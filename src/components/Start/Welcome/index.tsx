import { useTranslation } from 'react-i18next'
import Button from '@/components/elements/Button'
import ProviderMark from '@/components/elements/ProviderMark'
import { providerName, providerSlug, type Provider } from '@/api/provider'
import { useProviders } from '@/store'
import { LABEL } from '@/components/elements/tokens'
import { DiskGlyph } from '@/components/Main/icons'
import { isMobile } from '@/lib/platform'
import StepHeader from '../shared/StepHeader'
import RestoreAction from '../shared/RestoreAction'
import { COLUMN } from '../shared/layout'

interface Props {
  onFresh: () => void
  onConnect: (provider: Provider) => void
  onFile: () => void
}

// The first screen of the first run. One thing to press if this is a new
// device and nothing else exists yet; below the rule, the two places the app's
// own data can already be: one row per provider this build can connect, and
// a backup file. What restoring asks for is said on the screen that asks for
// it. A phone has no file system to hand a backup from, so there it is a cloud
// account or nothing.
export default function Welcome({ onFresh, onConnect, onFile }: Props) {
  const { t } = useTranslation()
  const providers = useProviders()

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

      {/* The rule sits midway between the button above and the rows below:
          the same air on both sides, and enough of it that the two groups read
          as two, not as one stack with a caption in it. */}
      <div className={`${COLUMN} mt-7 flex items-center gap-3`}>
        <span aria-hidden className="h-px flex-1 bg-line" />
        <span className={LABEL}>{t('Already using Rowel?')}</span>
        <span aria-hidden className="h-px flex-1 bg-line" />
      </div>

      <div className={`${COLUMN} mt-7 flex flex-col gap-2`}>
        {providers.map(provider => (
          <RestoreAction
            key={provider}
            mark={<ProviderMark provider={provider} size={18} />}
            title={t('Continue with {{provider}}', { provider: providerName(provider) })}
            body={t('Sync your data to this device.')}
            onClick={() => onConnect(provider)}
            testid={`start-${providerSlug(provider)}-button`}
          />
        ))}
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
