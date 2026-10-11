import { useStrength } from '@/hooks/useStrength'
import { useTranslation } from 'react-i18next'
import Meter from './Meter'
import { LEVEL_LABELS } from './levels'
import { META } from './tokens'

export default function StrengthBar({ password }: { password: string }) {
  const { t } = useTranslation()
  const strength = useStrength(password)
  if (!password) return null
  const score = strength?.score ?? null
  return (
    <div className="flex flex-none items-center gap-2.5">
      <Meter level={score} />
      <span className={META}>
        {score !== null ? t(LEVEL_LABELS[score]) : ''}
      </span>
    </div>
  )
}
