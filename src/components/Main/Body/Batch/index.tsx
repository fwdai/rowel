import Bar from './Bar'
import { usePickedRows } from './usePickedRows'

// Mounted with the list column, up only while a batch is: mounting is what
// pops the bar in, and what holds its keys to the life of the batch.
export default function BatchBar() {
  const rows = usePickedRows()
  return rows.length > 1 ? <Bar rows={rows} /> : null
}
