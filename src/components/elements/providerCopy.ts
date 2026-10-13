import type { TFunction } from 'i18next'
import { providerName, providerOpensBrowser, type Provider } from '@/api/provider'

/**
 * What to say while a connect is out, wherever one is drawn (the first run's
 * account screen, Settings › Sync, Workspaces › Restore). An OAuth provider's
 * connect is consent in a browser the user has to come back from, so the copy
 * waits on the provider by name; iCloud's is a check of the account the OS
 * already holds, which only the app is waiting on, so it says that instead.
 * With no provider named yet — the backend's `pending` can arrive before any
 * press named one — the browser is still the right guess, since only a
 * consent flow reaches that state that way.
 */
export const waitingFor = (t: TFunction, provider: Provider | null): string => {
  if (!provider) return t('Waiting for the browser…')
  const name = { provider: providerName(provider) }
  return providerOpensBrowser(provider)
    ? t('Waiting for {{provider}}…', name)
    : t('Checking your {{provider}} account…', name)
}
