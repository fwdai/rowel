import { useEffect, useState } from 'react'
import { describeError } from '@/api/errors'
import { orgDevices, orgLockboxes, type OrgDevice, type OrgLockbox } from '@/api/org'
import { useOrg } from '@/store'

export interface Listed<T> {
  items: T[]
  /** Why the list could not be read, or null; the last good list stays beside it. */
  error: string | null
}

/**
 * One of a connection's lists, read from Rust when the card mounts, again when
 * Rust says something changed (`org:changed`), and again on `tick` — the
 * card's own "I just changed it". `load` is the API function itself, so the
 * effect's dependencies are the plain values that name a read.
 */
const useList = <T,>(
  load: (url: string) => Promise<T[]>,
  url: string,
  seq: number,
  tick: number
): Listed<T> => {
  const [listed, setListed] = useState<Listed<T>>({ items: [], error: null })
  useEffect(() => {
    let live = true
    load(url).then(
      items => {
        if (live) setListed({ items, error: null })
      },
      (error: unknown) => {
        if (live) setListed(last => ({ items: last.items, error: describeError(error) }))
      }
    )
    return () => {
      live = false
    }
  }, [load, url, seq, tick])
  return listed
}

/**
 * The devices and lockboxes a server lists for the member, read separately:
 * a device still waiting for approval is told its devices but refused its
 * lockboxes, and the one answer should not hide the other.
 */
export const useDetails = (url: string) => {
  const seq = useOrg(state => state.seq)
  const [tick, setTick] = useState(0)
  const devices = useList<OrgDevice>(orgDevices, url, seq, tick)
  const lockboxes = useList<OrgLockbox>(orgLockboxes, url, seq, tick)
  const reload = () => setTick(count => count + 1)
  return { devices, lockboxes, reload }
}
