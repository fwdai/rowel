import { useTranslation } from 'react-i18next'
import { ENTROPY_LABELS } from '@/services/generator'
import { cx } from '@/utils/cx'
import IconButton from '@/components/elements/IconButton'
import Meter from '@/components/elements/Meter'
import { META } from '@/components/elements/tokens'
import { RefreshGlyph } from '@/components/Main/icons'

interface Props {
  value: string
  bits: number
  level: number
  onRegenerate: () => void
}

// Digits in the accent, symbols in warn, letters in plain ink, so the mix the
// defaults produce reads at a glance.
const inkOf = (char: string) =>
  /\d/.test(char) ? 'text-accent' : /[a-z]/i.test(char) ? 'text-text' : 'text-warn'

// Consecutive characters of one class share a span.
const runs = (value: string) =>
  [...value].reduce<{ ink: string; text: string }[]>((out, char) => {
    const ink = inkOf(char)
    const last = out[out.length - 1]
    if (last?.ink === ink) last.text += char
    else out.push({ ink, text: char })
    return out
  }, [])

// What the generator defaults make: a live sample on the field ground at the
// top of the card, with its entropy underneath.
export default function Sample({ value, bits, level, onRegenerate }: Props) {
  const { t } = useTranslation()
  return (
    <div className="bg-field px-4 py-3.5 inset-shadow-hairline">
      <div className="flex items-start gap-3">
        <div
          data-testid="settings-generator-sample"
          className="min-w-0 flex-1 text-md leading-relaxed tracking-secret break-all tabular-nums"
        >
          {runs(value).map((run, index) => (
            <span key={index} className={run.ink}>
              {run.text}
            </span>
          ))}
        </div>
        <IconButton
          label={t('Regenerate')}
          onClick={onRegenerate}
          testid="settings-generator-regenerate"
          className="border border-line2 hover:border-accent-line any-pointer-coarse:h-11 any-pointer-coarse:w-11"
        >
          <RefreshGlyph />
        </IconButton>
      </div>
      <div className="mt-2.5 flex items-center gap-2.5">
        <Meter level={level} />
        <span className={META}>
          {t(ENTROPY_LABELS[level])} · {bits} {t('bits')}
        </span>
        <span className={cx(META, 'ml-auto text-right')}>
          {t('Sample · new items start here')}
        </span>
      </div>
    </div>
  )
}
