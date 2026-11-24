import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { usePrefs, setPref } from '@/store'
import { SHORTCUTS, assignShortcut, hintOf, refusal, shortcutAt, type Shortcut } from '@/lib/shortcuts'
import { useShortcutKeys } from '@/hooks/useShortcutKeys'
import SettingsRow from '@/components/elements/SettingsRow'
import KeyCapture from './KeyCapture'

// One action: its name, and its key as a button that records a new one. A key
// another action already has is refused rather than taken from it.
export default function ShortcutRow({ id }: { id: Shortcut }) {
  const { t } = useTranslation()
  const keys = useShortcutKeys()
  const label = t(SHORTCUTS[id].label)
  // Why the last key pressed here was refused.
  const [note, setNote] = useState<string>()

  const assign = (code: string) => {
    const key = hintOf(code)
    const holder = shortcutAt(keys, code)
    const refused = refusal(id, code)
    if (refused === 'invalid') return setNote(t('Use a letter key'))
    if (refused === 'system') return setNote(t('{{key}} belongs to the system', { key }))
    if (refused === 'editing') return setNote(t('{{key}} is kept for editing text', { key }))
    if (holder && holder !== id)
      return setNote(
        t('{{key}} is already used by “{{action}}”', { key, action: t(SHORTCUTS[holder].label) })
      )
    setNote(undefined)
    setPref('shortcuts', assignShortcut(usePrefs.getState().shortcuts, id, code))
  }

  return (
    <SettingsRow
      label={label}
      description={note}
      control={
        <KeyCapture
          hint={hintOf(keys[id])}
          label={label}
          onCapture={assign}
          testid={`settings-shortcut-${id}`}
        />
      }
    />
  )
}
