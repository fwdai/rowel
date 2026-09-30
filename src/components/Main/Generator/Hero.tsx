import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import CopyButton from '@/components/elements/CopyButton'
import IconButton from '@/components/elements/IconButton'
import { CARD } from '@/components/elements/tokens'
import { RefreshGlyph } from '../icons'
import { Entropy } from './Output'

interface Props {
  value: string
  bits: number
  level: number
  // False while a draw is in flight: the value shown is the one being replaced.
  ready: boolean
  onRegenerate: () => void
}

// The desktop card's headline: the secret at display size, the two things to
// do with it at its edge, and how strong it is underneath. Sans, not mono: the
// mono tier is kept for raw key material (see theme.css).
export default function Hero({ value, bits, level, ready, onRegenerate }: Props) {
  const { t } = useTranslation()
  return (
    <div className={cx(CARD, 'p-4')}>
      <div className="flex items-start gap-3">
        <div
          data-testid="generator-output"
          className="min-h-8 min-w-0 flex-1 text-xl leading-8 tracking-secret break-all"
        >
          {value}
        </div>
        <div className="mt-0.5 flex flex-none gap-0.5">
          <IconButton title={t('Regenerate')} testid="generator-regenerate" onClick={onRegenerate}>
            <RefreshGlyph />
          </IconButton>
          <CopyButton value={value} title={t('Copy')} disabled={!ready} />
        </div>
      </div>
      <Entropy bits={bits} level={level} stretch className="mt-3.5" />
    </div>
  )
}
