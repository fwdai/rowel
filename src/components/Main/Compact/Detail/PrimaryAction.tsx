import { useTranslation } from 'react-i18next'
import type { Entry, EntryMeta } from '@/api/types'
import { cx } from '@/utils/cx'
import { usePrimaryAction } from '../../Body/Aside/Show/usePrimaryAction'
import { CheckGlyph, CopyGlyph } from '../../icons'
import { ACTION_BUTTON, ACTION_FILL, PRIMARY_ACTION } from '../chrome'

interface Props {
  entry: EntryMeta
  /** The decrypted entry, or null while `revealEntry` is still in flight. */
  revealed: Entry | null
}

/**
 * What the screen is for, where a thumb is: the desktop header's primary copy
 * action, moved to the bottom edge. Same hook, so it copies the same secret,
 * flashes the same check and answers the same ⏎.
 *
 * The fade above it is the content dissolving into the surface rather than
 * being cut off by an opaque bar — the button floats, so the rows have to be
 * seen to pass under it.
 */
export default function PrimaryAction({ entry, revealed }: Props) {
  const { t } = useTranslation()
  const { label, secret, copied, copy } = usePrimaryAction(entry, revealed)

  return (
    <>
      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-28 bg-linear-to-b from-transparent to-screen" />
      <button
        type="button"
        data-testid="primary-action-button"
        // Until the reveal lands there is nothing to copy, and a button that
        // copies '' is worse than one that is plainly not ready yet.
        disabled={!secret}
        onClick={copy}
        className={cx(ACTION_BUTTON, ACTION_FILL, PRIMARY_ACTION, !secret && 'opacity-50')}
      >
        {copied ? (
          <>
            <CheckGlyph />
            {t('Copied')}
          </>
        ) : (
          <>
            {/* Every kind's primary action is a copy, so the glyph says so. */}
            <CopyGlyph size={18} />
            {t(label)}
          </>
        )}
      </button>
    </>
  )
}
