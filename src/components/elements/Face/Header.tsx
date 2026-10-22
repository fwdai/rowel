import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { LABEL_TYPE } from '../tokens'

interface Props {
  /** What the face is of, untranslated: "Fingerprint", "Secret key". */
  label: TKey
  /**
   * A fact about it, already spelled, as a chip at the row's end — the
   * algorithm and size of a key, how many characters a token runs to.
   */
  chip?: string
}

// The head of a plate or a card: what it is, in the label face and the second
// ink, and what it says about itself in passing as a hairline chip in the mono
// tier a step under the captions. Wraps rather than clips, so on a narrow plate
// the chip drops under the label.
export default function FaceHeader({ label, chip }: Props) {
  const { t } = useTranslation()

  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className={`flex-1 ${LABEL_TYPE} text-(--face-ink2)`}>{t(label)}</span>
      {chip && (
        <span className="rounded-xs border border-(--face-rule) px-1.5 py-0.5 font-mono text-2xs tracking-label text-(--face-ink2)">
          {chip}
        </span>
      )}
    </div>
  )
}
