import type { ComponentProps } from 'react'
import { useTranslation } from 'react-i18next'
import { PROVIDERS, providerName, providerSlug, type Provider } from '@/api/provider'
import Button from './Button'
import ProviderMark from './ProviderMark'

type ButtonProps = ComponentProps<typeof Button>

interface Props {
  onPick: (provider: Provider) => void
  /**
   * Test id base: `<testid>-<slug>-connect` per provider, so Google's keeps the
   * `-drive-connect` every id had before there was a choice.
   */
  testid: string
  variant?: ButtonProps['variant']
  size?: ButtonProps['size']
  block?: boolean
  disabled?: boolean
  /** The row's own layout; side by side, wrapping, unless told otherwise. */
  className?: string
}

/**
 * One "Connect <provider>" per provider, for every place a vault or a restore
 * picks where to sync. One account per workspace, so this is a choice made once
 * per connect rather than a setting.
 */
export default function ProviderButtons({
  onPick,
  testid,
  variant = 'pale',
  size = 'md',
  block,
  disabled,
  className = 'flex flex-wrap gap-2'
}: Props) {
  const { t } = useTranslation()
  return (
    <div className={className}>
      {PROVIDERS.map(provider => (
        <Button
          key={provider}
          variant={variant}
          size={size}
          block={block}
          disabled={disabled}
          onClick={() => onPick(provider)}
          testid={`${testid}-${providerSlug(provider)}-connect`}
        >
          <ProviderMark provider={provider} size={14} />
          {t('Connect {{provider}}', { provider: providerName(provider) })}
        </Button>
      ))}
    </div>
  )
}
