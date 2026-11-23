import { useTranslation } from 'react-i18next'
import { useApp, useProviders, connectSync } from '@/store'
import { providerName, providerSlug } from '@/api/provider'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import ProviderMark from '@/components/elements/ProviderMark'
import SettingsRow from '@/components/elements/SettingsRow'
import { CARD, LABEL } from '@/components/elements/tokens'

// Before a vault syncs anywhere: one row per provider this build can connect,
// each with its own Connect. One account per vault, so this is a choice made
// once, and the account's card takes this list's place from the press on.
export default function Providers() {
  const { t } = useTranslation()
  const providers = useProviders()
  const inProgress = useApp(state => state.sync.inProgress)

  return (
    <div data-testid="settings-providers">
      <div className={cx(LABEL, 'mb-2')}>{t('Choose a provider')}</div>
      <div className={CARD}>
        {providers.map(provider => (
          <SettingsRow
            key={provider}
            icon={<ProviderMark provider={provider} size={16} />}
            label={providerName(provider)}
            control={
              <Button
                variant="pale"
                size="md"
                disabled={inProgress}
                onClick={() => connectSync(provider)}
                testid={`settings-${providerSlug(provider)}-connect`}
              >
                {/* The row names the provider for the eye; a screen reader
                    lands on the button alone, so it is named there too. */}
                {t('Connect')} <span className="sr-only">{providerName(provider)}</span>
              </Button>
            }
          />
        ))}
      </div>
    </div>
  )
}
