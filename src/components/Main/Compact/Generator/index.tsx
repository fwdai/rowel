import { useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import IconButton from '@/components/elements/IconButton'
import { cx } from '@/utils/cx'
import Panel from '../../Generator/Panel'
import Tabs from '../../Generator/Tabs'
import { useGeneratorDialog } from '../../Generator/useGeneratorDialog'
import { RefreshGlyph } from '../../icons'
import {
  ACTION_BUTTON,
  HEADER_ACTION,
  ROOT_ACTION,
  ROOT_CLEARANCE,
  ROOT_HEADER
} from '../chrome'
import Heading from '../Heading'

/**
 * The generator as a tab root rather than a dialog: a large title, the mode
 * switch, the same body the card shows, and one accent button where a thumb is.
 *
 * Only the standalone open lands here — opened from a password row the
 * generator carries somewhere to put the value, and is an overlay over the
 * screen that asked (see `Generator/Attached`).
 */
export default function Generator() {
  const { t } = useTranslation()
  // A tab root has nothing to close: the tab bar is the way out, and confirming
  // is not a reason to leave. Copying leaves the value on screen to copy again;
  // saving a keypair pushes the form over this screen, which is where the eye
  // goes next anyway.
  const stay = useCallback(() => {}, [])
  const generator = useGeneratorDialog(null, null, stay)
  const { mode, setMode, ready, regenerate, confirm, confirmLabel } = generator

  return (
    // `relative`: what the bottom action is pinned to.
    <div
      data-testid="generator-screen"
      className="relative flex min-h-0 flex-1 flex-col animate-fade bg-screen pt-[env(safe-area-inset-top)]"
    >
      <div className={`${ROOT_HEADER} px-4`}>
        <Heading title={t('Generator')} />
        {/* The header action tier, not the card's bordered 36px tile: in the
            corner of a root it is one of the sort, sync and menu buttons, and
            has to look it. */}
        <IconButton
          title={t('Regenerate')}
          testid="generator-regenerate"
          onClick={regenerate}
          className={HEADER_ACTION}
        >
          <RefreshGlyph />
        </IconButton>
      </div>

      <div className={cx('min-h-0 flex-1 overflow-y-auto px-4 pt-4', ROOT_CLEARANCE)}>
        {/* Nothing is waiting for a password here, so a keypair is on offer.
            Full width and 44px tall — it is the screen's own switch, not a
            control tucked beside a dialog title. */}
        <Tabs
          mode={mode}
          ssh
          onChange={setMode}
          className="w-full [&>button]:h-11 [&>button]:flex-1"
        />
        <div className="mt-4">
          <Panel generator={generator} />
        </div>
      </div>

      <button
        type="button"
        data-testid="generator-use-button"
        disabled={!ready}
        onClick={confirm}
        className={cx(ACTION_BUTTON, ROOT_ACTION, !ready && 'opacity-50')}
      >
        {t(confirmLabel)}
      </button>
    </div>
  )
}
