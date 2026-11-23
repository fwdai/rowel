import type { TFunction } from 'i18next'
import { providerName, type Provider } from '@/api/provider'
import type { AccountStanding } from '@/lib/workspace'

/**
 * A lockbox's standing with the account, as its row says it. One on this
 * device is named where it lives in the words Settings › Lockboxes gives the
 * same lockbox (the provider, or this device), so the two pages say one thing
 * of it. One only in the account says what it is missing.
 */
export const standingWords = (
  t: TFunction,
  standing: AccountStanding,
  provider: Provider
): string => {
  switch (standing) {
    case 'both':
      return providerName(provider)
    case 'device':
      return t('This device')
    case 'account':
      return t('Not on this device yet')
  }
}
