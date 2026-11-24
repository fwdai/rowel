import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import BiometricRow from '@/components/Main/Sidebar/Settings/Sections/Security/BiometricRow'
import type { BiometricMode, BiometryType } from '@/api/types'
import { seedApp } from './utils'

// The Android build: no Face ID or Touch ID there, and a device with several
// sensors is named by none of them.
vi.mock('@/lib/platform', () => ({
  isIOS: false,
  isAndroid: true,
  isMobile: true,
  isPC: false,
  chord: (key: string) => `⌘${key}`
}))

const show = (type: BiometryType, mode: BiometricMode | null) => {
  seedApp({ biometric: { available: mode !== null, canEnroll: true, type, mode } })
  render(<BiometricRow />)
}

describe('biometric unlock, on Android', () => {
  it.each([
    ['touch', 'Unlock with Fingerprint'],
    ['face', 'Unlock with Face unlock'],
    ['none', 'Unlock with Biometrics']
  ] as const)('names a %s gate the way Android does', (type, label) => {
    show(type, null)
    expect(screen.getByTestId('settings-biometric-toggle')).toHaveAccessibleName(label)
  })

  it('describes the Keystore gate once enrolled behind it', () => {
    show('touch', 'keystore-key')
    expect(screen.getByText(/protected by the Android Keystore/)).toBeInTheDocument()
  })
})
