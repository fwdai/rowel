import { useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { keyCode } from '@/utils/keys'

interface Props {
  // The chip for the current key (`⌘K`).
  hint: string
  // The action's name, for the button's accessible name.
  label: string
  // The physical key pressed (`KeyJ`).
  onCapture: (code: string) => void
  testid?: string
}

const MODIFIERS = ['Meta', 'Control', 'Shift', 'Alt']

/**
 * A shortcut's key as a button: click it and the next key pressed is the new
 * one. ⌘/Ctrl is implied, so a bare letter and the full chord both count.
 * Escape, a click or leaving the button gives up; Tab still moves on.
 */
export default function KeyCapture({ hint, label, onCapture, testid }: Props) {
  const { t } = useTranslation()
  const [recording, setRecording] = useState(false)

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (!recording || event.key === 'Tab') return
    // The press is the answer, not a command: no chord or Escape may reach the
    // app's listeners or the dialog's.
    event.preventDefault()
    event.stopPropagation()
    if (MODIFIERS.includes(event.key)) return
    setRecording(false)
    if (event.key !== 'Escape') onCapture(keyCode(event.nativeEvent))
  }

  return (
    <button
      type="button"
      aria-label={t('Shortcut for {{action}}', { action: label })}
      aria-pressed={recording}
      data-testid={testid}
      onClick={() => setRecording(!recording)}
      onBlur={() => setRecording(false)}
      onKeyDown={onKeyDown}
      className={cx(
        'min-w-[88px] cursor-pointer rounded-sm border px-2.5 py-1 text-center text-sm tabular-nums transition-colors',
        recording
          ? 'border-accent bg-accent-soft text-accent'
          : 'border-line2 bg-field hover:border-accent-line'
      )}
    >
      {recording ? t('Press a key…') : hint}
    </button>
  )
}
