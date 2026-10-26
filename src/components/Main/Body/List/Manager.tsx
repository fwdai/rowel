import { useLayoutEffect, useMemo } from 'react'
import type { EntryMeta } from '@/api/types'
import { useUi, keepPicked } from '@/store'
import { useLayout } from '@/hooks/useLayout'
import Item from './Item'
import Section from './Section'
import ListEmpty from '../Empty'
import { pickByClick } from './pick'
import { useVisibleEntries, useGrouped } from './useVisibleEntries'

const sections = (entries: EntryMeta[]): EntryMeta[][] => {
  const runs: EntryMeta[][] = []
  for (const entry of entries) {
    const run = runs[runs.length - 1]
    if (run && run[0].type === entry.type) run.push(entry)
    else runs.push([entry])
  }
  return runs
}

export default function Manager() {
  const entries = useVisibleEntries()
  const grouped = useGrouped()
  const door = useUi(state => state.view === 'items')
  const order = useMemo(() => entries.map(entry => entry.id), [entries])
  // Batches are the wide shell's: the phone has no modifier keys to pick
  // with, and no bar to act on what was picked.
  const batch = useLayout() === 'wide'
  // This is the one place that knows what the list shows, so it is the one
  // that holds a batch to it — after a query, a filter or a sync alike. A
  // layout effect, so a frame with rows picked out of sight is never painted.
  useLayoutEffect(() => keepPicked(order), [order])

  if (entries.length === 0) return <ListEmpty />

  const runs = grouped ? sections(entries) : []
  const captioned = runs.length > 1
  const item = (entry: EntryMeta) => (
    <Item
      entry={entry}
      key={entry.id}
      onClick={batch ? event => pickByClick(entry.id, order, event) : undefined}
    />
  )

  return (
    <div className="pb-6">
      {captioned
        ? runs.map(run => (
            <Section key={run[0].type} type={run[0].type} count={run.length} door={door}>
              {run.map(item)}
            </Section>
          ))
        : entries.map(item)}
    </div>
  )
}
