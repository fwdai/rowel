import { useEffect, useSyncExternalStore } from 'react'

/**
 * A backend image lookup, shared by every component showing the same key: one
 * call per key per session (the backend caches in the vault), fanned out to
 * all of them. The hook returns the data: URI, or null while loading and after
 * a miss — callers fall back to a glyph either way. `reset` empties it, for
 * when the session that filled it ends.
 */
export function imageLookup(fetchImage: (key: string) => Promise<string | null>) {
  const cache = new Map<string, string | null>()
  const pending = new Set<string>()
  const listeners = new Set<() => void>()

  const subscribe = (listener: () => void) => {
    listeners.add(listener)
    return () => {
      listeners.delete(listener)
    }
  }

  const lookup = (key: string) => {
    if (cache.has(key) || pending.has(key)) return
    pending.add(key)
    fetchImage(key)
      .catch(() => null)
      .then(uri => {
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
    cache.clear()
    pending.clear()
  }

  return { useImage, reset }
}
