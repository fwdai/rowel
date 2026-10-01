import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { clearPasswordHistory } from '@/store'
import { cx } from '@/utils/cx'
import { META_TYPE } from '../../tokens'

// Forgetting every previous password, armed by the first press and done by
// the second — the editor's Cancel guard, on one button. Nothing to undo
// afterwards, so it asks; and nothing to show on success, because the reveal
// that follows comes back without a history and this whole block goes with it.
export default function Clear({ id }: { id: string }) {
  const { t } = useTranslation()
  const [armed, setArmed] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const press = () => {
    if (!armed) {
      setArmed(true)
      return
    }
    setArmed(false)
    setError(null)
    clearPasswordHistory(id).catch(() =>
      setError(t('Could not clear the history. Please try again.'))
    )
  }

  return (
    <div className="mt-1.5 flex items-center justify-between gap-3">
      {error ? <span className={`${META_TYPE} text-bad`}>{error}</span> : <span />}
      <button
        type="button"
        data-testid="password-history-clear"
        onClick={press}
        className={cx(
          META_TYPE,
          'cursor-pointer rounded-sm px-1 transition-colors any-pointer-coarse:min-h-11',
          armed ? 'text-bad hover:brightness-110' : 'text-text2 hover:text-text'
        )}
      >
        {armed ? t('Clear all previous passwords?') : t('Clear history')}
      </button>
    </div>
  )
}
