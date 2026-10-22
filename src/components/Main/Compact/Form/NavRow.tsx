import { useTranslation } from 'react-i18next'
import type { Draft } from '../../Body/Aside/Show/Edit/useDraft'
import NavBar from '../NavBar'
import NavButton from '../NavButton'

interface Props {
  draft: Draft
  /** What this screen is for: "Add a login", or the entry being edited. */
  title: string
}

/**
 * The form screen's nav row: Cancel on the left, Save on the right, the iOS way
 * round.
 *
 * Cancel is the draft's own guarded exit rather than a second way out — the
 * first press arms it and says so, the second discards. That wording is also
 * why the desktop's unsaved dot is not here: a Cancel that reads "Discard
 * changes?" already tells you there is something to lose.
 */
export default function NavRow({ draft, title }: Props) {
  const { t } = useTranslation()

  return (
    <NavBar
      title={title}
      leading={
        // Same element for both presses: arm, then discard.
        <NavButton
          testid="cancel-entry-button"
          danger={draft.confirmDiscard}
          onClick={draft.cancel}
        >
          {draft.confirmDiscard ? t('Discard changes?') : t('Cancel')}
        </NavButton>
      }
      trailing={
        <NavButton
          testid="save-entry-button"
          primary
          trailing
          disabled={draft.saving}
          onClick={draft.save}
        >
          {t('Save')}
        </NavButton>
      }
    />
  )
}
