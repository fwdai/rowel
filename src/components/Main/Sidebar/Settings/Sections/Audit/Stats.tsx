import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { Audit } from '@/api/tools'
import { usePrefs, setView, closeSettings } from '@/store'
import { auditCounts } from '@/utils/vaultScore'
import { cx } from '@/utils/cx'
import { LABEL, TEXT_LINK } from '@/components/elements/tokens'
import { setBreachCheck } from './breach'

const LINK = `mt-1 text-left text-base ${TEXT_LINK}`

const openHealth = () => {
  setView('health')
  closeSettings()
}

interface CellProps {
  label: string
  // Undefined when there is no answer to give: nothing scanned yet, or a
  // monitor that is switched off.
  value: number | undefined
  children: ReactNode
}

// A label over its figure, the geometry the Drive card's strip draws too.
function Cell({ label, value, children }: CellProps) {
  const tone = value === undefined ? 'text-text2' : value > 0 ? 'text-warn' : 'text-good'
  return (
    <div className="flex flex-col items-start px-5 py-3.5 not-first:border-l not-first:border-line">
      <div className={LABEL}>{label}</div>
      <div className={cx('mt-0.5 text-xl font-semibold tabular-nums tracking-display', tone)}>
        {value ?? '—'}
      </div>
      {children}
    </div>
  )
}

// The hero's strip: weak, reused and breached, each with the way to the
// entries behind it. Breached has nothing to count while monitoring is off, so
// it offers to turn it on instead.
export default function Stats({ audit }: { audit: Audit | null }) {
  const { t } = useTranslation()
  const breachCheck = usePrefs(state => state.breachCheck)
  const counts = audit ? auditCounts(audit) : null

  const review = (testid: string) => (
    <button type="button" data-testid={testid} onClick={openHealth} className={LINK}>
      {t('Review')}
    </button>
  )

  return (
    <div
      data-testid="settings-audit-counts"
      className="relative grid grid-cols-3 border-t border-line"
    >
      <Cell label={t('weak')} value={counts?.weak}>
        {review('settings-open-health')}
      </Cell>
      <Cell label={t('reused')} value={counts?.reused}>
        {review('settings-open-health-reused')}
      </Cell>
      <Cell label={t('breached')} value={breachCheck ? counts?.breached : undefined}>
        {breachCheck ? (
          review('settings-open-health-breached')
        ) : (
          <button
            type="button"
            data-testid="settings-breach-enable"
            onClick={() => setBreachCheck(true)}
            className={LINK}
          >
            {t('Turn on monitoring')}
          </button>
        )}
      </Cell>
    </div>
  )
}
