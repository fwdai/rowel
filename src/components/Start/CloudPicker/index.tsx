import { useTranslation } from 'react-i18next'
import ProviderMark from '@/components/elements/ProviderMark'
import { providerName, providerSlug, type Provider } from '@/api/provider'
import { useProviders } from '@/store'
import StepHeader from '../shared/StepHeader'
import RestoreAction from '../shared/RestoreAction'
import { COLUMN } from '../shared/layout'

interface Props {
  onPick: (provider: Provider) => void
}

// One row per provider this build can connect, behind the welcome screen's
// single "Continue with a cloud account" row. The welcome offers three ways in
// and stays that way however many providers there are; which clouds exist is
// this screen's to list, and the build's to say (`useProviders`). Each row
// starts the consent with its own provider — the backend has no other way to
// know whose sign-in page to open.
export default function CloudPicker({ onPick }: Props) {
  const { t } = useTranslation()
  const providers = useProviders()

  return (
    <>
      <StepHeader
        eyebrow={t('Already using Rowel?')}
        title={t('Which cloud do you sync through?')}
        body={t('Pick the account your other device syncs to.')}
      />

      <div className={`${COLUMN} mt-8 flex flex-col gap-2`}>
        {providers.map(provider => (
          <RestoreAction
            key={provider}
            mark={<ProviderMark provider={provider} size={18} />}
            title={t('Continue with {{provider}}', { provider: providerName(provider) })}
            body={t('Sync your data to this device.')}
            onClick={() => onPick(provider)}
            testid={`start-${providerSlug(provider)}-button`}
          />
        ))}
      </div>
    </>
  )
}
