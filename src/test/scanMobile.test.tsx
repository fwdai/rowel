import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import { clearSession, useUi, useVault } from '@/store'
import { withEntries, loginMeta, seedApp } from './utils'
import { calls, mockCommand } from './ipc'

// The whole module graph reads as a phone, so the picker's scan row is the
// phone's: it opens the system's own sheet through a file input, and scans
// whatever that hands back from its bytes.
vi.mock('@/lib/platform', () => ({
  isIOS: true,
  isAndroid: false,
  isMobile: true,
  isPC: false,
  chord: (key: string) => `⌘${key}`
}))

const CARD = { number: '4242424242424242', month: '04', year: '27', name: 'ADA LOVELACE' }

const BYTES = new Uint8Array([0xff, 0xd8, 0xff, 0xe0])
const PHOTO = new File([BYTES], 'IMG_0042.jpg', { type: 'image/jpeg' })

// Open the Add picker, with its scan row on screen.
const openPicker = async () => {
  withEntries([loginMeta({ id: 'l1', title: 'Google' })])
  seedApp({ scanSupported: true })
  render(<Main />)
  await userEvent.click(screen.getByTestId('add-entry-button'))
}

const input = () => screen.getByTestId<HTMLInputElement>('add-scan-input')

const closed = () => expect(screen.queryByTestId('add-secret-modal')).not.toBeInTheDocument()

beforeEach(() => vi.clearAllMocks())

describe('scanning on a phone', () => {
  it('opens the system sheet through a file input, not a sheet of its own', async () => {
    await openPicker()
    const opened = vi.spyOn(input(), 'click')

    await userEvent.click(screen.getByTestId('add-scan-image'))

    expect(opened).toHaveBeenCalled()
    // The system's sheet offers the camera itself: no `capture`, which would
    // skip it and open the camera outright.
    expect(input()).toHaveAttribute('accept', 'image/*')
    expect(input()).not.toHaveAttribute('capture')
    // Over the picker, not instead of it: a dismissed sheet lands back here.
    expect(screen.getByTestId('add-secret-modal')).toBeInTheDocument()
    expect(calls('pick_file')).toHaveLength(0)
  })

  it('scans the photo from its bytes and hands over to the editor', async () => {
    mockCommand('scan_image_bytes', () => ({ kind: 'card', fields: CARD }))
    await openPicker()

    await userEvent.upload(input(), PHOTO)

    await waitFor(() => expect(useVault.getState().creating).toBe('card'))
    // The editor took the prefill: the number is in it, grouped as the card
    // field prints it.
    await waitFor(() =>
      expect(document.querySelector('input[name="number"]')).toHaveValue('4242 4242 4242 4242')
    )
    closed()
    expect(calls('scan_image')).toHaveLength(0)
    expect(calls('scan_image_bytes')).toEqual([BYTES])
  })

  it('reports a photo it could not even load, since the picker is gone', async () => {
    const broken = new File([BYTES], 'IMG_0043.jpg', { type: 'image/jpeg' })
    Object.defineProperty(broken, 'arrayBuffer', {
      value: () => Promise.reject(new Error('the file went away'))
    })
    await openPicker()

    await userEvent.upload(input(), broken)

    await waitFor(() => expect(useUi.getState().scanError).toBe('failed'))
    expect(useUi.getState().scanBusy).toBe(false)
    closed()
    expect(calls('scan_image_bytes')).toHaveLength(0)
  })

  it('drops the photo when the session that took it ends during the read', async () => {
    let finish: (bytes: ArrayBuffer) => void = () => {}
    const slow = new File([BYTES], 'IMG_0044.jpg', { type: 'image/jpeg' })
    Object.defineProperty(slow, 'arrayBuffer', {
      value: () => new Promise<ArrayBuffer>(resolve => (finish = resolve))
    })
    mockCommand('scan_image_bytes', () => ({ kind: 'card', fields: CARD }))
    await openPicker()
    await userEvent.upload(input(), slow)
    expect(useUi.getState().scanBusy).toBe(true)

    // A lock mid-read, and another workspace unlocked after it.
    act(() => clearSession())
    withEntries([loginMeta({ id: 'w2', title: 'Other workspace' })])
    await act(async () => finish(BYTES.buffer))

    expect(calls('scan_image_bytes')).toHaveLength(0)
    expect(useVault.getState().creating).toBeNull()
    expect(useUi.getState()).toMatchObject({ scanBusy: false, scanError: null })
  })

  it('reports a photo nothing could be read from', async () => {
    await openPicker()

    await userEvent.upload(input(), PHOTO)

    await waitFor(() => expect(useUi.getState().scanError).toBe('unreadable'))
    expect(useVault.getState().creating).toBeNull()
  })
})
