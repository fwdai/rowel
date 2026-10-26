import type { TFunction } from 'i18next'
import type { ChipTone } from '@/components/elements/Chip'
import type { OrgDeviceState } from '@/api/org'

/** A device's state as a chip reads it, and the ink that state takes. */
export const deviceStateWords = (t: TFunction, state: OrgDeviceState): string => {
  switch (state) {
    case 'approved':
      return t('Enrolled')
    case 'pending':
      return t('Waiting for approval')
    case 'revoked':
      return t('Revoked')
  }
}

export const deviceStateTone = (state: OrgDeviceState): ChipTone =>
  state === 'approved' ? 'good' : state === 'revoked' ? 'bad' : 'neutral'
