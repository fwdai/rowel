import { mockCommand } from './ipc'

export const exchangeReport = (total: number, dryRun: boolean) => ({
  total,
  imported: dryRun ? 0 : total,
  skipped: 0,
  duplicates: 0,
  dryRun,
  errors: [],
  warnings: [],
  entries: []
})

/**
 * The backend's half of a transfer from another app, answering as Rust does:
 * one transfer held at a time, by its arrival number, until it is imported or
 * dropped; a preview or an import of any other number refused. A spec says
 * what arrives and what the lock does, and `exchange_held` and
 * `import_exchange` follow.
 */
export const exchangeBackend = () => {
  const rows = new Map<number, number>()
  let held: number | null = null
  mockCommand('exchange_held', () => held)
  mockCommand('import_exchange', args => {
    const dryRun = args.dryRun === true
    if (args.arrival !== held)
      return Promise.reject({
        kind: 'other',
        message: 'that transfer is no longer waiting; export from the other app again'
      })
    if (!dryRun) held = null
    return exchangeReport(rows.get(args.arrival as number) ?? 0, dryRun)
  })
  return {
    /** A transfer of `total` rows arrives, replacing whatever was held. */
    hold(arrival: number, total = 3) {
      rows.set(arrival, total)
      held = arrival
    },
    /** What a lock does to it. */
    drop() {
      held = null
    }
  }
}
