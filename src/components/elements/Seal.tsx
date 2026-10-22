import { useTranslation } from 'react-i18next'
import { LockGlyph } from '@/components/Main/icons'

// What stands in for a body while it is sealed. Fixed: the seal says nothing
// about how long the secret is.
const SEAL = Array.from({ length: 4 }, () => '•'.repeat(64))

/**
 * A blurred stand-in rendered in a secret body's place, with the
 * press-to-reveal plate over it. The caller owns the `shown` state and swaps
 * this out for the real body once it is pressed; the container must be
 * `relative`, so the plate has something to cover. The plate names the gesture
 * the device has: a click under a pointer, a tap where a finger is one.
 */
export default function Seal({ onReveal, testid }: { onReveal: () => void; testid?: string }) {
  const { t } = useTranslation()

  return (
    <>
      <div aria-hidden className="select-none break-all text-text2 blur-[5px]">
        {SEAL.map((line, index) => (
          <div key={index}>{line}</div>
        ))}
      </div>
      <button
        type="button"
        onClick={onReveal}
        aria-label={t('Reveal')}
        data-testid={testid}
        className="absolute inset-0 grid cursor-pointer place-items-center"
      >
        <span className="flex h-[30px] items-center gap-2 rounded-sm border border-line2 bg-detail px-3 font-sans text-base font-medium tracking-[0] whitespace-nowrap text-text shadow-[0_6px_18px_rgba(0,0,0,0.12)]">
          <LockGlyph size={13} />
          <span className="any-pointer-coarse:hidden">{t('Sealed · click to reveal')}</span>
          <span className="hidden any-pointer-coarse:inline">{t('Sealed · tap to reveal')}</span>
        </span>
      </button>
    </>
  )
}
