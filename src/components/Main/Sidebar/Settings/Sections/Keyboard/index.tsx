import { useTranslation } from 'react-i18next'
import { usePrefs, setPref } from '@/store'
import { SHORTCUT_IDS } from '@/lib/shortcuts'
import Button from '@/components/elements/Button'
import { CARD } from '@/components/elements/tokens'
import ShortcutRow from './ShortcutRow'

// Settings › Keyboard shortcuts, desktop only: every ⌘/Ctrl + letter chord.
// Stored in this device's settings file and never synced — a key that suits
// one keyboard can clash on another.
export default function Keyboard() {
  const { t } = useTranslation()
  const changed = usePrefs(state => Object.keys(state.shortcuts).length > 0)

  return (
    <>
      <div className={`${CARD} mb-7`}>
        {SHORTCUT_IDS.map(id => (
          <ShortcutRow key={id} id={id} />
        ))}
      </div>
      <Button
        variant="pale"
        size="md"
        disabled={!changed}
        onClick={() => setPref('shortcuts', {})}
        testid="settings-shortcuts-reset"
      >
        {t('Restore defaults')}
      </Button>
    </>
  )
}
