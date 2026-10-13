import { useTranslation } from 'react-i18next'
import { useApp, openSettings } from '@/store'
import { cx } from '@/utils/cx'
import Tooltip from '@/components/elements/Tooltip'
import { CloudGlyph, DiskGlyph, CheckGlyph, AlertGlyph } from '../../icons'
import { syncView, type SyncTone } from './state'

// The badge tints, by tone. Only the two settled states carry one -- `local`,
// `idle` and `loading` have no verdict to report, and the legacy indicator hid
// its badge in exactly those cases too.
const BADGE: Partial<Record<SyncTone, string>> = {
  good: 'bg-good text-detail',
  bad: 'bg-bad text-detail'
}

// Sync chip: a round 28px well holding one glyph that says where the vault
// lives -- a disk when sync is off, a cloud when it is on -- with the run's
// outcome riding on a corner badge and a ring that orbits while a sync is in
// flight. This is the legacy Electron indicator's language (chip · centre glyph
// · corner tick/exclamation · spinner ring) rebuilt on the current tokens, so
// the centre glyph never changes with status: position is one fact, progress is
// another, and swapping the glyph mid-run would conflate them.
//
// It is always mounted, including on a vault that has never synced -- "your
// changes are on this disk and nowhere else" is a standing property of the
// vault worth a permanent slot, not noise. Clicking goes where the state is
// owned: Settings › Sync & backup, which holds Connect and Sync now.
//
// How to get there is the shell's to say. The default opens the wide modal on
// that section; a shell whose settings are a screen of its own (the phone's)
// passes its own way of pushing the same pane -- the store's `settingsSection`
// is the modal's nav selection and means nothing there.
interface Props {
  className?: string
  onClick?: () => void
}

export default function SyncIndicator({ className, onClick }: Props) {
  // The hook, not the bare `t` -- only it re-renders the chip when the language
  // changes, and the chip can sit untouched in the chrome for a whole session.
  const { t } = useTranslation()
  const sync = useApp(state => state.sync)
  const { tone, message, detail } = syncView(sync)
  // The backend's message wins when there is one; it is already prose, in no
  // catalogue, and translating it is not on offer.
  const label = detail ?? t(message)

  const Glyph = tone === 'local' ? DiskGlyph : CloudGlyph
  const badge = BADGE[tone]

  return (
    <Tooltip content={label}>
      <button
        type="button"
        data-testid="sync-indicator"
        data-tone={tone}
        aria-label={label}
        onClick={onClick ?? (() => openSettings('sync'))}
        className={cx(
          'relative grid h-7 w-7 flex-none cursor-pointer place-items-center rounded-full text-text2 transition-colors hover:bg-hover hover:text-text',
          className
        )}
      >
        {/* The orbiting ring. Three sides at a quarter strength and one at full
            reads as a comet rather than a rotating circle -- the same trick the
            legacy spinner used, and the same motion as the unlock card's orbit.
            Inset so it rides just outside the glyph. */}
        {tone === 'loading' && (
          <span
            aria-hidden
            className="absolute -inset-px animate-spin rounded-full border-2 border-accent/25 border-l-accent"
          />
        )}

        {/*
          Optical, not geometric. All three glyphs are already centred on the
          same axis -- disk, cloud and lock each have their ink centred on y=12
          of a 24 viewBox -- but the lock's weight is not: it is a thin shackle
          over a solid body, so its mass sits in its lower half and it reads
          about 2 viewBox units low. Beside it, an evenly weighted disk or cloud
          reads high. This nudges the pair back onto the lock's apparent centre
          rather than its true one; it is half the computed difference, which is
          one device pixel at 2x.
        */}
        <Glyph className="translate-y-[0.5px]" />

        {badge && (
          <span
            aria-hidden
            className={cx(
              // The separating ring is `app`, not `chrome`: chrome is a
              // translucent wash, so a ring in it would let the glyph read
              // through the gap it exists to create.
              'absolute -right-0.5 -bottom-0.5 grid h-3 w-3 place-items-center rounded-full ring-2 ring-app',
              badge
            )}
          >
            {tone === 'good' ? <CheckGlyph size={8} /> : <AlertGlyph size={8} />}
          </span>
        )}
      </button>
    </Tooltip>
  )
}
