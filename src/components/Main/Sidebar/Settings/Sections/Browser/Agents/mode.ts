import type { TFunction } from 'i18next'
import type { GrantMode } from '@/api/browser'

/**
 * A row's mode in words, for the modes a fill row does not need spelling
 * out: a header rule or grant is a secret sent in requests, an env one a
 * secret in a command's environment. A fill (and a passkey) says nothing —
 * it is what the list has always shown.
 */
export const modeWords = (t: TFunction, mode: GrantMode): string | null => {
  switch (mode) {
    case 'header':
      return t('as a request header')
    case 'env':
      return t("in a command's environment")
    default:
      return null
  }
}
