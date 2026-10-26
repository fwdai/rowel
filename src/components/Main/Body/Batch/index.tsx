import { useVault, selectBatch } from '@/store'
import Bar from './Bar'

// Mounted with the list column, up exactly while the store says a batch is
// (`selectBatch`, the test every other surface uses too): mounting is what
// pops the bar in, and what holds its keys — Esc among them — to the life of
// the batch.
export default function BatchBar() {
  const batch = useVault(selectBatch)
  return batch ? <Bar /> : null
}
