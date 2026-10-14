import type { EntryType } from '@/api/types'
import { kindOf } from '@/kinds'
import { openAddPickerAt, startEntry } from '@/store'

/**
 * Start a new entry of `type` from a surface that already knows the kind — the
 * palette's "Add an identity", a filtered list's empty pane. A kind with a
 * second question (which document) is asked it first, on the picker's own
 * page; every other kind opens its form at once, as before.
 */
export const addEntry = (type: EntryType) =>
  kindOf(type).choices ? openAddPickerAt(type) : startEntry(type)
