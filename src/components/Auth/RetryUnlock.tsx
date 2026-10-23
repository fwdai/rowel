import { useTranslation } from 'react-i18next'
import Button from '@/components/elements/Button'

interface Props {
  /** `useUnlock`'s retry: nothing is drawn while there is nothing to retry. */
  onRetry?: () => void
  disabled: boolean
}

/**
 * Under the card, once a lockbox has been moved onto the vault password and
 * the unlock after it failed: open it again without asking for anything.
 */
export default function RetryUnlock({ onRetry, disabled }: Props) {
  const { t } = useTranslation()
  if (!onRetry) return null
  return (
    <div className="mt-4 flex justify-center">
      <Button variant="pale" testid="unlock-retry-button" disabled={disabled} onClick={onRetry}>
        {t('Try again')}
      </Button>
    </div>
  )
}
