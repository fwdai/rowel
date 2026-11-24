import { useEffect } from 'react'
import { onBackButtonPress } from '@tauri-apps/api/app'
import { leaveApp } from '@/lib/android'
import { isAndroid } from '@/lib/platform'
import { goBack } from './useBack'

/**
 * Android's Back — the button and the edge gesture — as the app's own way
 * back: whatever is on top takes the press (`useBack`), and at the root, where
 * nothing does, the app is left.
 *
 * Listening takes the press away from the system for good: Tauri's activity
 * only falls back to its own handling (the webview's history, then the
 * activity's default) while no listener is registered, and it has no way to
 * hand a single press back. So leaving is asked for by name. It puts the task
 * in the background rather than finishing the activity — what Android itself
 * has done with Back on a launcher's root since 12 — so the session is under
 * the same auto-lock clock as after Home, and coming back is not a cold start.
 *
 * The keyboard is not this hook's: while it is up the system spends the press
 * on dismissing it and the app never hears of it.
 *
 * Mounted once, by `App`. A no-op everywhere but Android: no other platform
 * has the button, and the listener's command exists only there.
 */
export function useBackButton() {
  useEffect(() => {
    if (!isAndroid) return
    let alive = true
    let unregister: (() => void) | undefined

    onBackButtonPress(() => {
      if (!goBack()) leaveApp()
    })
      .then(listener => {
        const stop = () => void listener.unregister().catch(() => {})
        if (alive) unregister = stop
        else stop()
      })
      .catch(() => {})

    return () => {
      alive = false
      unregister?.()
    }
  }, [])
}
