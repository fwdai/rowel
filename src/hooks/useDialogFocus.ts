import { useEffect, type RefObject } from 'react'
import { useDialogPresence } from './useDialogPresence'

const TABBABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

// A control taken out of the tab order (a dialog header's pointer-only close)
// is not where focus should start or wrap either.
const tabbables = (root: HTMLElement | null) =>
  Array.from(root?.querySelectorAll<HTMLElement>(TABBABLE) ?? []).filter(el => el.tabIndex >= 0)

/**
 * The keyboard contract every dialog frame shares — the centered `Modal` and
 * the full-screen `Sheet` alike.
 *
 * A dialog owns the keyboard while it is up: focus starts inside it, Tab cycles
 * within it, Escape closes it, and focus goes back to whatever opened it on
 * close — otherwise the tab order resumes at the top of the document, behind
 * the dialog. Only the topmost `[role="dialog"]` reacts, so a stacked one (the
 * generator over Settings) is neither closed from underneath by Escape nor has
 * its focus pulled back down by the trap.
 *
 * A frame that traps focus is a modal surface, so it is registered as one here
 * (`isModalOpen`), which is what tells the accelerators behind it to stand down.
 */
export function useDialogFocus(dialog: RefObject<HTMLElement | null>, onClose: () => void) {
  useDialogPresence()

  useEffect(() => {
    const trigger = document.activeElement as HTMLElement | null
    const first = tabbables(dialog.current)[0]
    if (first) first.focus()
    else dialog.current?.focus()
    return () => trigger?.focus()
  }, [dialog])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      // Something nearer the key already handled it (a field's own Esc, the
      // generator's own listener) — acting again would close twice.
      if (e.defaultPrevented) return
      const dialogs = document.querySelectorAll('[role="dialog"]')
      if (dialogs[dialogs.length - 1] !== dialog.current) return
      // Escape closes every dialog — the scrim and the X are pointer-only exits.
      if (e.key === 'Escape') {
        e.preventDefault()
        return onClose()
      }
      // Arrow keys are left alone: the add-secret picker steers its grid with
      // them.
      if (e.key !== 'Tab') return
      const all = tabbables(dialog.current)
      if (all.length === 0) return
      const edge = e.shiftKey ? all[0] : all[all.length - 1]
      const inside = dialog.current?.contains(document.activeElement)
      if (!inside || document.activeElement === edge) {
        e.preventDefault()
        ;(e.shiftKey ? all[all.length - 1] : all[0]).focus()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [dialog, onClose])
}
