import { useTranslation } from 'react-i18next'
import { providerName, providerSlug, type Provider } from '@/api/provider'
import { useProviders } from '@/store'
import { cx } from '@/utils/cx'
import Button from './Button'
import ProviderMark from './ProviderMark'
import SettingsRow from './SettingsRow'
import { CARD, LABEL } from './tokens'

interface Props {
  onPick: (provider: Provider) => void
  /**
   * Test id base: the list itself, and `<testid>-<slug>-connect` per provider,
   * so Google's keeps the `-drive-connect` every id had before there was a
   * choice.
   */
  testid: string
  /** The list's own id, where it is not `testid`. */
  listTestid?: string
  disabled?: boolean
}

/**
 * The choice of a cloud provider in Settings: one row per provider this build
 * can connect, each with its mark, its name and its own quiet Connect. One
 * list for every page that asks (connecting this vault under Sync & backup,
 * restoring a lockbox from an account under Lockboxes), so the same question
 * looks the same wherever it is asked. The first run asks it as its step's
 * own buttons instead (`ProviderButtons`).
 */
export default function ProviderList({ onPick, testid, listTestid = testid, disabled }: Props) {
  const { t } = useTranslation()
  const providers = useProviders()

  return (
    <div data-testid={listTestid}>
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
                disabled={disabled}
                onClick={() => onPick(provider)}
                testid={`${testid}-${providerSlug(provider)}-connect`}
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
