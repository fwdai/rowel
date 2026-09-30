import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import Meter from '@/components/elements/Meter'
import { LEVEL_INK } from '@/components/elements/levels'
import { ENTROPY_LABELS } from '@/services/generator'
import { wellClass } from '@/components/elements/formStyles'
import { META_TYPE } from '@/components/elements/tokens'

interface Props {
  value: string
  bits: number
  level: number
}

// The meter and its reading, under the secret wherever the secret is drawn.
export function Entropy({
  bits,
  level,
  stretch,
  className
}: Omit<Props, 'value'> & { stretch?: boolean; className?: string }) {
  const { t } = useTranslation()
  return (
    <div className={cx('flex items-center gap-2.5', className)}>
      <Meter level={level} stretch={stretch} />
      <span className={cx(META_TYPE, LEVEL_INK[level])}>
        {t(ENTROPY_LABELS[level])} · {bits} {t('bits')}
      </span>
    </div>
  )
}

// The generated secret on its field tile, with the entropy meter underneath.
export default function Output({ value, bits, level }: Props) {
  return (
    <>
      <div
        data-testid="generator-output"
        className={`min-h-14 ${wellClass} p-4 text-lg leading-relaxed tracking-secret break-all`}
      >
        {value}
      </div>
      <Entropy bits={bits} level={level} className="mt-3" />
    </>
  )
}
