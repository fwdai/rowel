import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import { clearSession, useUi, useVault } from '@/store'
import { withEntries, loginMeta, seedApp } from './utils'
import { calls, mockCommand } from './ipc'

// The whole module graph reads as a phone, so the picker's scan row is the
// phone's: it asks where the photo comes from before anything opens.
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

// Open the Add picker and answer its scan row, which swaps in the source sheet.
const openSheet = async () => {
  withEntries([loginMeta({ id: 'l1', title: 'Google' })])
  seedApp({ scanSupported: true })
  render(<Main />)
  await userEvent.click(screen.getByTestId('add-entry-button'))
  await userEvent.click(screen.getByTestId('add-scan-image'))
}

const closed = () => expect(screen.queryByTestId('scan-source-sheet')).not.toBeInTheDocument()

const stillOpen = () => expect(screen.getByTestId('scan-source-sheet')).toBeInTheDocument()

beforeEach(() => vi.clearAllMocks())

describe('scanning on a phone', () => {
  it('asks where the photo comes from, in the picker’s place', async () => {
    await openSheet()

    const sheet = screen.getByTestId('scan-source-sheet')
    expect(sheet).toHaveTextContent('Take Photo')
    expect(sheet).toHaveTextContent('Photo Library')
    expect(sheet).toHaveTextContent('Choose File…')
    // In place of the picker, not over it: a sheet never stacks on a sheet.
    expect(screen.queryByTestId('add-secret-modal')).not.toBeInTheDocument()
    expect(calls('pick_file')).toHaveLength(0)
    // The camera row opens the camera outright, not the system's own sheet.
    expect(screen.getByTestId('scan-source-capture')).toHaveAttribute('capture', 'environment')
  })

  it('backs out of the question to nothing', async () => {
    await openSheet()

    await userEvent.keyboard('{Escape}')

    closed()
    expect(screen.queryByTestId('add-secret-modal')).not.toBeInTheDocument()
  })

  it('drops the question with the session it was asked in', async () => {
    await openSheet()

    act(() => clearSession())

    closed()
    expect(useUi.getState().scanSource).toBe(false)
  })

  describe('the camera', () => {
    it('opens on the row', async () => {
      await openSheet()
      const input = screen.getByTestId<HTMLInputElement>('scan-source-capture')
      const opened = vi.spyOn(input, 'click')

      await userEvent.click(screen.getByTestId('scan-source-camera'))

      expect(opened).toHaveBeenCalled()
      expect(calls('pick_file')).toHaveLength(0)
    })

    it('scans the photo it took from its bytes and hands over to the editor', async () => {
      mockCommand('scan_image_bytes', () => ({ kind: 'card', fields: CARD }))
      await openSheet()

      await userEvent.upload(screen.getByTestId('scan-source-capture'), PHOTO)

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

    it('reports a photo it could not even load, since the sheet is gone', async () => {
      const broken = new File([BYTES], 'IMG_0043.jpg', { type: 'image/jpeg' })
      Object.defineProperty(broken, 'arrayBuffer', {
        value: () => Promise.reject(new Error('the file went away'))
      })
      await openSheet()

      await userEvent.upload(screen.getByTestId('scan-source-capture'), broken)

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
      await openSheet()
      await userEvent.upload(screen.getByTestId('scan-source-capture'), slow)
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
      await openSheet()

      await userEvent.upload(screen.getByTestId('scan-source-capture'), PHOTO)

      await waitFor(() => expect(useUi.getState().scanError).toBe('unreadable'))
      expect(useVault.getState().creating).toBeNull()
    })
  })

  describe('the library', () => {
    it('is the same granted dialog as the desktop, run from Rust', async () => {
      mockCommand('pick_file', () => '/var/mobile/tmp/IMG_0042.heic')
      mockCommand('scan_image', () => ({ kind: 'card', fields: CARD }))
      await openSheet()

      await userEvent.click(screen.getByTestId('scan-source-library'))

      await waitFor(() => expect(useVault.getState().creating).toBe('card'))
      closed()
      expect(calls('pick_file')).toEqual([{ kind: 'image', label: 'Images' }])
      expect(calls('scan_image')).toContainEqual({ path: '/var/mobile/tmp/IMG_0042.heic' })
      expect(calls('scan_image_bytes')).toHaveLength(0)
    })

    it('leaves the question open when the picker is cancelled', async () => {
      mockCommand('pick_file', () => null)
      await openSheet()

      await userEvent.click(screen.getByTestId('scan-source-library'))

      stillOpen()
      expect(calls('scan_image')).toHaveLength(0)
    })
  })

  describe('Files', () => {
    it('is the same dialog, asked for the file browser under the image filter', async () => {
      mockCommand('pick_file', () => '/private/var/mobile/Documents/passport.png')
      mockCommand('scan_image', () => ({ kind: 'card', fields: CARD }))
      await openSheet()

      await userEvent.click(screen.getByTestId('scan-source-browse'))

      await waitFor(() => expect(useVault.getState().creating).toBe('card'))
      closed()
      expect(calls('pick_file')).toEqual([{ kind: 'image', label: 'Images', browse: true }])
    })
  })
})
