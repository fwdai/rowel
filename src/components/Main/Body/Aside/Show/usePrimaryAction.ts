import { useEffect } from 'react'
import type { Entry, EntryMeta } from '@/api/types'
import type { TKey } from '@/i18n'
import { kindOf } from '@/kinds'
import { isModalOpen, useVault } from '@/store'
import { useCopied } from '@/hooks/useCopied'

// Enter is the detail's accelerator for the primary action, but only as a bare
// press outside any interactive control or open dialog — anywhere else the key
// already belongs to whatever holds focus.
const isPlainEnter = (e: KeyboardEvent) =>
  e.key === 'Enter' &&
  !e.metaKey &&
  !e.ctrlKey &&
  !e.altKey &&
  !e.shiftKey &&
  !e.defaultPrevented

// Anything that owns Enter itself: a field the user is typing in, or a control
// Enter already activates (a chip, the sort button, a menu item). Copying on
// top of those would fire two actions from one press.
const inInteractive = (target: EventTarget | null) =>
  target instanceof Element &&
  !!target.closest(
    'input, textarea, select, [contenteditable="true"], button, a, [role="button"], [role="menuitem"]'
  )

export interface PrimaryAction {
  /** Untranslated label for the button ("Copy password"). */
  label: TKey
  /** The headline secret, or '' while the reveal is in flight — the disabled test. */
  secret: string
  /** Up for a short beat after a copy, so the button can flash its check. */
  copied: boolean
  copy: () => void
}

/**
 * The one thing an entry offers in a single press, wherever the shell puts it:
 * the desktop header's copy button and the phone's bottom action are the same
 * action drawn twice. The ⏎ accelerator is installed here rather than by either
 * button, so exactly one of them is mounted and exactly one listener exists.
 *
 * The secret is read straight off the already-decrypted entry, so copying never
 * touches a row's on-screen reveal state.
 */
export function usePrimaryAction(entry: EntryMeta, revealed: Entry | null): PrimaryAction {
  const { copied, copy } = useCopied()
  const kind = kindOf(entry.type)
  const secret = revealed ? kind.primarySecret(revealed) : ''

  useEffect(() => {
    if (!secret) return
    const onKeyDown = (e: KeyboardEvent) => {
      if (!isPlainEnter(e) || inInteractive(e.target) || isModalOpen()) return
      // The desktop pane keeps an entry up while the next one is decrypted;
      // for that moment the selection has moved on, and so has the key.
      const selected = useVault.getState().currentId
      if (selected && selected !== entry.id) return
      e.preventDefault()
      copy(secret)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [secret, copy, entry.id])

  return { label: kind.primaryActionLabel, secret, copied, copy: () => copy(secret) }
}
