import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { EntryMeta } from '@/api/types'
import {
  useUi,
  useVault,
  clearPicked,
  deleteEntries,
  purgeEntries,
  isModalOpen,
  showNotice
} from '@/store'
import EscClose from '@/components/elements/EscClose'
import Actions from './Actions'

export type Run = (op: (ids: string[]) => Promise<void>) => void

const typing = (target: EventTarget | null) =>
  target instanceof Element && !!target.closest('input, textarea, [contenteditable="true"]')

// The batch's action bar, floated over the foot of the list column: the
// pointer is already there, picking rows, so the verbs come to it rather than
// sit across the window. The count, the verbs, and the way out.
export default function Bar({ rows }: { rows: EntryMeta[] }) {
  const { t } = useTranslation()
  const picked = useVault(state => state.picked)
  const archived = useUi(state => state.view === 'archive')
  // Armed for this pick and no other: picking anything else disarms.
  const [armedFor, setArmedFor] = useState<ReadonlySet<string> | null>(null)
  const [busy, setBusy] = useState(false)
  const armed = armedFor === picked
  const count = t('{{count}} selected', { count: rows.length })
  // Armed, the question takes the count's place rather than the button's:
  // the button keeps its width and the bar never has to truncate either.
  const prompt = armed
    ? archived
      ? t('Delete forever?')
      : t('Archive {{count}} items?', { count: rows.length })
    : count

  const run: Run = op => {
    setArmedFor(null)
    setBusy(true)
    op(rows.map(row => row.id))
      .catch(() => showNotice(t('Some items could not be changed. Please try again.')))
      .finally(() => setBusy(false))
  }

  // Two presses, like every destructive verb in the app: the first arms, the
  // second runs. Archive in a live view; in the Archive, the last delete.
  const destroy = () => (armed ? run(archived ? purgeEntries : deleteEntries) : setArmedFor(picked))

  // Window-wide while the bar is up, so the keys work wherever focus is: on a
  // row, on one of these buttons, or nowhere. Esc backs out one step at a
  // time — the armed verb first, then the batch.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (isModalOpen() || busy) return
      if (event.key === 'Escape') {
        if (armed) setArmedFor(null)
        else clearPicked()
        return
      }
      const chord = event.metaKey || event.ctrlKey
      const del = (chord && event.key === 'Backspace') || event.key === 'Delete'
      if (!del || typing(event.target)) return
      event.preventDefault()
      destroy()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  })

  return (
    <div
      role="toolbar"
      aria-label={count}
      data-testid="batch-bar"
      className="animate-pop absolute inset-x-3 bottom-3 z-20 flex h-11 items-center gap-1.5 rounded-xl border border-line bg-detail pr-1.5 pl-3.5 text-text shadow-float"
    >
      <span
        data-testid="batch-count"
        aria-live="polite"
        className="min-w-0 flex-1 truncate text-base font-medium tabular-nums"
      >
        {prompt}
      </span>
      <Actions
        rows={rows}
        archived={archived}
        armed={armed}
        busy={busy}
        run={run}
        destroy={destroy}
      />
      <span aria-hidden="true" className="mx-0.5 h-4 w-px flex-none bg-line" />
      <EscClose
        onClose={clearPicked}
        disabled={busy}
        testid="batch-clear"
        className="h-7 rounded-sm px-1.5 text-text2 hover:bg-hover hover:text-text"
      />
    </div>
  )
}
