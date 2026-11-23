import { useEffect } from 'react'
import { forgetAccount, useApp } from '@/store'

/**
 * A sign-in the user walked away from goes with the screen that asked for it.
 * Leaving the screen is backing out, whether by Back, another section or
 * closing Settings, so the account a probe left pending is forgotten and no
 * later create, restore or connect can adopt tokens nobody is looking at.
 *
 * Read as the screen goes, not as it last drew: the probe answers by event,
 * after any render that could have captured it. Only when there is one to
 * forget, so leaving a screen where no sign-in started costs no backend call.
 *
 * Harmless mid-restore: the backend holds the account for the restore's
 * length and refuses to drop it, so what the user asked for still finishes.
 */
export function useForgetAccountOnLeave() {
  useEffect(
    () => () => {
      if (useApp.getState().setupAccount.status !== 'idle') forgetAccount()
    },
    []
  )
}
