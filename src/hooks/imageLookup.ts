import { useEffect, useSyncExternalStore } from 'react'

/**
 * A backend image lookup, shared by every component showing the same key: one
 * call per key per session (the backend caches in the vault), fanned out to
 * all of them. The hook returns the data: URI, or null while loading and after
 * a miss — callers fall back to a glyph either way. `reset` empties it, for
 * when the session that filled it ends.
 *
 * A lookup still in flight when it is reset belongs to the session that ended:
 * each one remembers the generation it was asked in, and an answer from an
 * older one is dropped — it neither lands in the new session's cache nor clears
 * the pending mark of a lookup the new session has made for the same key.
 */
export function imageLookup(fetchImage: (key: string) => Promise<string | null>) {
  const cache = new Map<string, string | null>()
  const pending = new Set<string>()
  const listeners = new Set<() => void>()
  let generation = 0

  const subscribe = (listener: () => void) => {
    listeners.add(listener)
    return () => {
      listeners.delete(listener)
    }
  }

  const lookup = (key: string) => {
    if (cache.has(key) || pending.has(key)) return
    pending.add(key)
    const asked = generation
    fetchImage(key)
      .catch(() => null)
      .then(uri => {
        if (asked !== generation) return
        cache.set(key, uri ?? null)
        pending.delete(key)
        listeners.forEach(listener => listener())
      })
  }

  const useImage = (key?: string): string | null => {
    const image = useSyncExternalStore(subscribe, () => (key ? (cache.get(key) ?? null) : null))
    useEffect(() => {
      if (key) lookup(key)
    }, [key])
    return image
  }

  const reset = () => {
    generation += 1
    cache.clear()
    pending.clear()
  }

  return { useImage, reset }
}
