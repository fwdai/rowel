import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import Monogram from '@/components/elements/Monogram'
import { CARD, META } from '@/components/elements/tokens'

interface Props {
  name: string
  /** As the workspace carries it; Monogram falls back to the seed's hue. */
  color?: string | null
  /** The hue with no colour chosen: the workspace's id, or a stand-in for one to come. */
  seed: string
  meta: string
  testid: string
}

// The workspace as the list will draw it, redrawn as it is typed and picked.
export default function Preview({ name, color, seed, meta, testid }: Props) {
  const { t } = useTranslation()
  const label = name.trim()

  return (
    <div data-testid={testid} className={cx(CARD, 'flex items-center gap-4 p-4')}>
      <Monogram
        size={52}
        fontSize={20}
        color={color ?? undefined}
        seed={seed}
        name={label || '?'}
      />
      <div className="min-w-0 flex-1">
        <div
          className={cx(
            'truncate text-lg font-semibold tracking-display',
            label ? 'text-text' : 'text-text2'
          )}
        >
          {label || t('Untitled lockbox')}
        </div>
        <div className={cx(META, 'mt-0.5 truncate')}>{meta}</div>
      </div>
    </div>
  )
}
