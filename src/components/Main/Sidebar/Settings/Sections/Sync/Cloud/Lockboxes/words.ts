import type { TFunction } from 'i18next'
import { providerName, type Provider } from '@/api/provider'
import type { AccountStanding } from '@/lib/workspace'

export type StandingTone = 'good' | 'neutral'

/**
 * A lockbox's standing with the account, as its row says it. One on this
 * device is named where it lives in the words Settings › Lockboxes gives the
 * same lockbox (the provider, or this device), so the two pages say one thing
 * of it; the provider is lit, as the overview's row lights a connection. One
 * only in the account says what it is missing.
 */
export const standingWords = (
  t: TFunction,
  standing: AccountStanding,
  provider: Provider
): { label: string; tone: StandingTone } => {
  switch (standing) {
    case 'both':
      return { label: providerName(provider), tone: 'good' }
    case 'device':
      return { label: t('This device'), tone: 'neutral' }
    case 'account':
      return { label: t('Not on this device yet'), tone: 'neutral' }
  }
}
