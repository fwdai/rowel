import type { TFunction } from 'i18next'
import { nearbyDetailText, type NearbyDevice } from '@/api/nearby'

export interface StatusLine {
  text: string
  bad: boolean
}

const quiet = (text: string): StatusLine => ({ text, bad: false })
const bad = (text: string): StatusLine => ({ text, bad: true })

export const statusLine = (
  t: TFunction,
  device: NearbyDevice,
  ago: (iso: string) => string
): StatusLine => {
  switch (device.state) {
    case 'paused':
      return quiet(t('Paused'))
    case 'offline':
      return quiet(
        device.lastSyncedAt
          ? t('Not nearby · last synced {{when}}', { when: ago(device.lastSyncedAt) })
          : t('Not nearby')
      )
    case 'syncing':
      return quiet(t('Syncing…'))
    case 'waiting':
      return quiet(t('Waiting to sync'))
    case 'upToDate':
      return quiet(t('Up to date'))
    case 'needsPassword':
      return bad(t('Needs the new master password'))
    case 'clockSkew':
      return bad(nearbyDetailText(device) ?? t('The clocks on the two devices don’t match'))
    case 'error':
      return bad(nearbyDetailText(device) ?? t('Something went wrong'))
  }
}
