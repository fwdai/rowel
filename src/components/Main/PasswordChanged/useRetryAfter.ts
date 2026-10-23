import { useEffect, useState } from 'react'

/**
 * The seconds left on a `tooManyAttempts` backoff, ticking down once a second
 * to 0, the way the lock screen counts its own (`useUnlock`).
 */
export function useRetryAfter(): [number, (seconds: number) => void] {
  const [retryAfter, setRetryAfter] = useState(0)

  useEffect(() => {
    if (retryAfter <= 0) return
    const id = setTimeout(() => setRetryAfter(retryAfter - 1), 1000)
    return () => clearTimeout(id)
  }, [retryAfter])

  return [retryAfter, setRetryAfter]
}
