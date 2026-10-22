import { useState } from 'react'
import { useFields } from '@/components/elements/fields'
import { docTypeOf, TEMPLATES } from '../templates'
import type { Doc } from './DocCell'
import Document from './Document'

/**
 * The document as an object, in both modes.
 *
 * One face for every type (see `Document`), printing only the rows the
 * document's template has, so a value left over from another type (an import,
 * an older vault) never shows on a document that has no place for it. Editing,
 * the same cells take the typing, and the template says which of them the
 * document cannot be saved without.
 */
export default function Face() {
  const { entry, set } = useFields()
  const editing = !!set
  // Typing into dots is guesswork, so the editor starts revealed; the same
  // eye takes it back in either mode.
  const [shown, setShown] = useState(editing)
  const docType = docTypeOf(entry)
  const rows = new Map(TEMPLATES[docType].map(row => [row.key, !!row.required]))

  const doc: Doc = {
    value: key => {
      const raw = rows.has(key) ? entry[key] : ''
      if (typeof raw !== 'string') return ''
      // A draft keeps its spaces as they are typed; a read value is printed clean.
      return editing ? raw : raw.trim()
    },
    has: key => rows.has(key),
    set: editing ? (key, value) => set(key, value) : undefined,
    required: key => !!rows.get(key),
    shown,
    toggle: () => setShown(!shown)
  }

  return <Document docType={docType} doc={doc} />
}
