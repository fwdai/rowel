import { useTranslation } from 'react-i18next'
import type { Provider } from '@/api/provider'
import type { AccountStanding } from '@/lib/workspace'
import { standingWords } from './words'

interface Props {
  standing: AccountStanding
  /** The account's provider. */
  provider: Provider
  /** Said after the standing: what tells one pack in the account from another. */
  detail?: string
  testid: string
}

// A row's second line: where the lockbox lives. Plain words with no tone of
// their own: whether the account is in sync is the card's to say, above, and a
// lit row would claim it for a lockbox whose first sync has not landed. Under
// the name rather than beside it, so a phone's width goes to both.
export default function Standing({ standing, provider, detail, testid }: Props) {
  const { t } = useTranslation()
  const label = standingWords(t, standing, provider)

  return <span data-testid={testid}>{detail ? `${label} · ${detail}` : label}</span>
}
