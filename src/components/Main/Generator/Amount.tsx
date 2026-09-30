import { useTranslation } from 'react-i18next'
import {
  LENGTH_RANGE,
  WORDS_RANGE,
  type GeneratorSettings
} from '@/services/generator'
import SettingsRow from '@/components/elements/SettingsRow'
import { LABEL } from '@/components/elements/tokens'

interface Props {
  settings: GeneratorSettings
  onChange: (patch: Partial<GeneratorSettings>) => void
  // As a settings row in the desktop card, rather than the phone's bare line.
  row?: boolean
}

// One slider row that measures characters in random mode and words in
// memorable mode — the only dimension that changes between the two.
export default function Amount({ settings, onChange, row }: Props) {
  const { t } = useTranslation()
  const byWords = settings.mode === 'memorable'
  const { min, max } = byWords ? WORDS_RANGE : LENGTH_RANGE
  const value = byWords ? settings.words : settings.length
  const label = t(byWords ? 'Words' : 'Length')

  const slider = (
    <>
      <input
        type="range"
        aria-label={label}
        data-testid="generator-amount"
        min={min}
        max={max}
        value={value}
        onChange={event => {
          const next = Number(event.target.value)
          onChange(byWords ? { words: next } : { length: next })
        }}
        className="h-1.5 flex-1 accent-accent"
      />
      <span className="w-[58px] flex-none text-right text-base tabular-nums text-text2">
        {value} {t(byWords ? 'words' : 'chars')}
      </span>
    </>
  )

  if (row)
    return (
      <SettingsRow
        label={label}
        control={<div className="flex w-64 items-center gap-3.5">{slider}</div>}
      />
    )

  return (
    <div className="mt-5 flex items-center gap-3.5">
      <span className={`w-[66px] flex-none ${LABEL}`}>{label}</span>
      {slider}
    </div>
  )
}
