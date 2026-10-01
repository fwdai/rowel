import { useState, useEffect, useMemo } from 'react'
import {
  useVault,
  setCurrentEntry,
  setNoEntry,
  saveEntry,
  clearPrefill,
  isModalOpen
} from '@/store'
import { kindOf } from '@/kinds'
import { pruneExtra } from '@/components/elements/fields'
import { mergeFields } from '@/components/Main/Scan/fields'
import type { DraftValue, EntryDraft } from '@/kinds/draft'
import type { Entry, EntryType } from '@/api/types'
import { t } from '@/i18n'

export interface Draft {
  model: EntryDraft
  /** Writes one draft key; the list shapes are the whole-collection blocks. */
  set: (name: string, value: DraftValue) => void
  /** Something has been typed since the draft was loaded. */
  dirty: boolean
  /** Save has been attempted, so required fields may now complain. */
  attempted: boolean
  /** Cancel is armed: the next request discards. */
  confirmDiscard: boolean
  /** A save is out with the backend; further requests are ignored until it lands. */
  saving: boolean
  saveError: string | null
  save: () => void
  cancel: () => void
}

/**
 * One editing session: the draft, its dirty baseline, the two-press discard
 * guard, and the ⌘⏎ / Esc bindings. Mounted only while editing and keyed on the
 * entry, so every session starts from a fresh read of the vault.
 */
export function useDraft(type: EntryType, revealed: Entry | null): Draft {
  const kind = kindOf(type)
  // `Show` holds the editor back until an existing entry's reveal has landed,
  // so the decrypted values are already here at mount. Seeding from them beats
  // adopting them in an effect, which raced whatever was typed first — and
  // means a later reveal (a sync merge moving `updatedAt`) cannot replace the
  // draft. The baseline is the state at open; a concurrent change resolves
  // through last-writer-wins on save.
  // Fields a scan read out of an image, waiting for a form to land in. A new
  // entry's are in the store before this mounts, so they are part of the
  // initial draft — the editor opens already filled in, and nothing about it
  // reads as unsaved typing.
  const [seed] = useState(() => useVault.getState().prefill)
  const initial = (): EntryDraft => mergeFields({ ...(revealed ?? kind.defaults) }, seed ?? {})

  const [attempted, setAttempted] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [confirmDiscard, setConfirmDiscard] = useState(false)
  const [saving, setSaving] = useState(false)
  const [model, setModel] = useState<EntryDraft>(initial)
  // What the model looked like when it was loaded — the dirty baseline.
  const [pristine] = useState<EntryDraft>(initial)

  // Compared serialized, once per edit rather than once per render: the draft
  // re-renders on every keystroke and the baseline never changes.
  const pristineJson = useMemo(() => JSON.stringify(pristine), [pristine])
  const dirty = useMemo(() => JSON.stringify(model) !== pristineJson, [model, pristineJson])

  const set = (name: string, value: DraftValue) => {
    setConfirmDiscard(false)
    setModel(current => ({ ...current, [name]: value }))
  }

  // A scan of what is already being edited arrives after the mount: it fills
  // this draft's blanks in place rather than opening a second one. Consumed
  // either way, so the prefill can never seed a later entry.
  const prefill = useVault(state => state.prefill)
  useEffect(() => {
    if (!prefill) return
    if (prefill !== seed) setModel(current => mergeFields(current, prefill))
    clearPrefill()
  }, [prefill, seed])

  const close = () => {
    if (model.id) setCurrentEntry(model.id)
    else setNoEntry()
  }

  // Esc / Cancel. Unsaved edits arm an inline confirm on the Cancel button
  // instead of a blocking dialog; the next request discards.
  const cancel = () => {
    if (!dirty || confirmDiscard) {
      close()
      return
    }
    setConfirmDiscard(true)
  }

  // One write at a time. A new entry has no id until the backend answers, so a
  // second press while the first is in flight minted a second id — and a second
  // row — for the same draft.
  const save = () => {
    if (saving) return
    if (!kind.isValid(model)) {
      setAttempted(true)
      return
    }
    setSaveError(null)
    setSaving(true)
    // The rotation stamp and the password history are the core's to write: it
    // compares the saved password with the stored one, so a password typed
    // back to what it was is no change (see `Entry::record_password_change`).
    // Never imply success on a failed write: surface the error, stay in edit.
    saveEntry(pruneExtra(model))
      .catch(() => setSaveError(t('Could not save. Please try again.')))
      .finally(() => setSaving(false))
  }

  // Bound fresh every render: both handlers close over the current draft.
  //
  // Both keys stand down while a dialog is up. The editor opens dialogs of its
  // own (the generator, off the password row), and this listener is on
  // `document` — so it sees Escape on the way *down* to the dialog's own
  // handler. Without the guard, dismissing the generator would also end the
  // edit session, and ⌘⏎ inside it would save the draft behind it.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (isModalOpen()) return
      if (event.key === 'Escape') return cancel()
      if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
        event.preventDefault()
        save()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  })

  return { model, set, dirty, attempted, confirmDiscard, saving, saveError, save, cancel }
}
