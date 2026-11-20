import type { TFunction } from 'i18next'
import { nearbyDetailText, type NearbyDevice } from '@/api/nearby'

export interface StatusLine {
  text: string
  /** The device needs something done before it syncs again. */
  bad: boolean
}

/**
 * What a device row says under its name. `ago` spells `lastSyncedAt` for a
 * sentence ("3 days ago"); it is the caller's, which knows the user's date
 * pattern. `clockSkew` and `error` say what the backend said: its kind in the
 * user's language when it sent one, else its own sentence.
 */
export const statusLine = (
  t: TFunction,
  device: NearbyDevice,
  ago: (iso: string) => string
): StatusLine => {
  switch (device.state) {
    case 'syncing':
      return { text: t('Syncing…'), bad: false }
    case 'upToDate':
      return { text: t('Up to date'), bad: false }
    case 'offline':
      return {
        text: device.lastSyncedAt
          ? t('Not nearby · last synced {{when}}', { when: ago(device.lastSyncedAt) })
          : t('Not nearby'),
        bad: false
      }
    case 'paused':
      return { text: t('Paused'), bad: false }
    case 'needsPassword':
      return { text: t('Needs the new master password'), bad: true }
    case 'clockSkew':
      return {
        text: nearbyDetailText(device) ?? t('The clocks on the two devices don’t match'),
        bad: true
      }
    case 'error':
      return { text: nearbyDetailText(device) ?? t('Something went wrong'), bad: true }
  }
}
