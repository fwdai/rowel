import { useTranslation } from 'react-i18next'
import Face from '@/components/elements/Face'

// A line of print not read yet.
const LINE = 'h-2 rounded-full bg-(--face-rule)'

/**
 * A scan under way, shown where its result will land: the document as a
 * blank — the photograph's frame, a few lines of print — with the scanner's
 * beam passing down it. The same face the filled-in entry is drawn on, so the
 * screen goes from "being read" to "read" without changing what it is showing.
 *
 * Only the picture and its two lines: the surface is the caller's — the wide
 * shell's detail pane (`Body/Aside`), the phone's screen (`Compact`).
 */
export default function Reading() {
  const { t } = useTranslation()

  return (
    <div
      data-testid="scan-reading"
      role="status"
      className="animate-fade relative flex flex-col items-center gap-7 text-center"
    >
      <Face
        tone="slate"
        aria-hidden
        className="aspect-[85.6/53.98] w-[320px] max-w-full"
      >
        <div className="flex h-full flex-col gap-4 p-5">
          <div className={`${LINE} w-2/5`} />
          <div className="flex min-h-0 flex-1 gap-4">
            <div className="aspect-[3/4] h-full flex-none rounded-[6px] bg-(--face-rule)" />
            <div className="flex flex-1 flex-col justify-center gap-3">
              <div className={`${LINE} h-3 w-4/5`} />
              <div className={`${LINE} w-3/5`} />
              <div className={`${LINE} w-2/3`} />
              <div className={`${LINE} w-2/5`} />
            </div>
          </div>
        </div>
        <div className="animate-scan pointer-events-none absolute inset-0 border-b-2 border-accent bg-linear-to-t from-accent/20 via-accent/5 via-25% to-transparent to-45%" />
      </Face>
      <div className="flex flex-col items-center gap-1.5">
        <strong className="shimmer-text text-lg font-semibold tracking-display">
          {t('Reading the image…')}
        </strong>
        <span className="max-w-sm text-base text-text2">
          {t('Read on this device. The image is never copied or uploaded.')}
        </span>
      </div>
    </div>
  )
}
