import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { EyeGlyph, EyeOffGlyph } from '@/components/Main/icons'

interface Props {
  shown: boolean
  onToggle: () => void
  /** The selector the e2e suite presses: `reveal-number`, `reveal-apiKey`. */
  testid: string
  className?: string
}

// The one control a face carries: the eye that lifts every mask on it at
// once. The 28px icon affordance (see elements/IconButton) in the face's own
// inks, so it reads as printed on the object rather than floating over it.
// Where a finger is one of the pointers the hit area grows to the 44px target
// through a pseudo-element, so the box keeps its place in the face's layout.
export default function Eye({ shown, onToggle, testid, className }: Props) {
  const { t } = useTranslation()
  const label = shown ? t('Hide') : t('Reveal')

  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={shown}
      data-testid={testid}
      onClick={onToggle}
      className={cx(
        "relative grid h-7 w-7 flex-none cursor-pointer place-items-center rounded-sm text-(--face-ink2) transition-colors hover:bg-(--face-hover) hover:text-(--face-ink) any-pointer-coarse:before:absolute any-pointer-coarse:before:-inset-2 any-pointer-coarse:before:content-['']",
        shown && 'bg-(--face-hover) text-(--face-ink)',
        className
      )}
    >
      {shown ? <EyeOffGlyph /> : <EyeGlyph />}
    </button>
  )
}
