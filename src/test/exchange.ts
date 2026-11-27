import { setExchange } from '@/store'
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
 * The backend's half of a transfer from another app, behaving as Rust does:
 * one transfer held at a time, numbered by the change that brought it, let go
 * of by its import or when its session ends; a preview or an import of any
 * other number refused; and every change told to the store, as `exchange:held`
 * would tell it. A spec says what arrives and what ends, and the rest follows.
 */
export const exchangeBackend = () => {
  const rows = new Map<number, number>()
  let held: number | null = null
  let change = 0
  const now = () => ({ arrival: held, change })
  const set = (next: number | null) => {
    change += 1
    held = next === null ? null : change
    return now()
  }
  mockCommand('exchange_held', now)
  mockCommand('import_exchange', args => {
    const dryRun = args.dryRun === true
    if (args.arrival !== held)
      return Promise.reject({
        kind: 'other',
        message: 'that transfer is no longer waiting; export from the other app again'
      })
    if (!dryRun) setExchange(set(null))
    return exchangeReport(rows.get(args.arrival as number) ?? 0, dryRun)
  })
  return {
    /** A transfer of `total` rows arrives and is announced. Answers its number. */
    arrive(total = 3) {
      const told = set(0)
      rows.set(told.arrival!, total)
      setExchange(told)
      return told.arrival!
    },
    /** One arrives with nothing listening yet: held, and only an ask finds it. */
    arriveUnheard(total = 3) {
      const told = set(0)
      rows.set(told.arrival!, total)
      return told.arrival!
    },
    /** The session it was offered to ends. */
    end() {
      if (held !== null) setExchange(set(null))
    }
  }
}
