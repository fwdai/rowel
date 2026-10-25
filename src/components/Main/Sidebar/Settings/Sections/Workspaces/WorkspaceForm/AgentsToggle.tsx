import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { META } from '@/components/elements/tokens'
import Toggle from '@/components/elements/Toggle'

interface Props {
  value: boolean
  onChange: (next: boolean) => void
  disabled?: boolean
  testidPrefix: string
}

// The lockbox's agent-access switch: off by default, and the one thing that
// decides whether a rule or a grant can ever name an item in it. The note is
// the plain-language consequence, since "agents" alone says nothing about
// what happens while the vault is open.
export default function AgentsToggle({ value, onChange, disabled, testidPrefix }: Props) {
  const { t } = useTranslation()
  const label = t('Agents can use items in this lockbox')

  return (
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0">
        <div className="text-base text-text">{label}</div>
        <div className={cx(META, 'mt-0.5')}>
          {t(
            'While {{appName}} is unlocked, an agent with a standing rule uses items here without asking.'
          )}
        </div>
      </div>
      <Toggle
        name="agents"
        checked={value}
        disabled={disabled}
        onChange={onChange}
        aria-label={label}
        testid={`${testidPrefix}-agents`}
      />
    </div>
  )
}
