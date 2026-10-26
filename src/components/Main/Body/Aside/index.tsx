import { useVault, useCurrentEntry } from '@/store'
import { useVariant } from '../Empty/variant'
import { DetailEmpty } from '../Empty'
import Summary from '../Batch/Summary'
import Show from './Show'
import Audit from './Audit'

// Detail-pane content. Creating and editing happen right here, in the pane:
// `Show` renders the kind's fields either way, read or write.
export default function Aside() {
  // The kind being created, or null. An edit takes its kind from the entry.
  const draftType = useVault(state => state.creating)
  const editing = useVault(state => state.editing)
  const batch = useVault(state => state.picked.size > 0)
  const entry = useCurrentEntry()
  const variant = useVariant()

  // Distinct keys: a draft and an entry must never share a `Show` instance, or
  // the draft inherits the entry's in-flight reveal (and with it, its id).
  if (draftType) return <Show key="draft" type={draftType} editing />
  if (batch) return <Summary />
  if (entry) return <Show key={entry.id} entry={entry} editing={editing} />
  // No variant means the health view has a score to show; everything else the
  // pane can be showing is one kind of empty or another.
  if (!variant) return <Audit />
  return <DetailEmpty variant={variant} />
}
