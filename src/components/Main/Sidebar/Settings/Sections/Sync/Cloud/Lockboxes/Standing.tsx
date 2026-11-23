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

// A row's second line: where the lockbox stands, as quiet as a SettingsNavRow's
// status, with the same dot where the tone has one. Under the name rather than
// beside it, so a phone's width goes to the name and the words both.
export default function Standing({ standing, provider, detail, testid }: Props) {
  const { t } = useTranslation()
  const { label, tone } = standingWords(t, standing, provider)

  return (
    <span data-testid={testid} data-tone={tone} className="flex items-center gap-1.5">
      {tone === 'good' && (
        <span aria-hidden className="h-1.5 w-1.5 flex-none rounded-full bg-good" />
      )}
      {detail ? `${label} · ${detail}` : label}
    </span>
  )
}
