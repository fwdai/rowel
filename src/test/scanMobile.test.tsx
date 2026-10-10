import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import { useUi, useVault } from '@/store'
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

// Open the Add picker and answer its scan row, which raises the source sheet.
const openSheet = async () => {
  withEntries([loginMeta({ id: 'l1', title: 'Google' })])
  seedApp({ scanSupported: true })
  render(<Main />)
  await userEvent.click(screen.getByTestId('add-entry-button'))
  await userEvent.click(screen.getByTestId('add-scan-image'))
}

const closed = () => {
  expect(screen.queryByTestId('scan-source-sheet')).not.toBeInTheDocument()
  expect(screen.queryByTestId('add-secret-modal')).not.toBeInTheDocument()
}

const stillOpen = () => {
  expect(screen.getByTestId('scan-source-sheet')).toBeInTheDocument()
  expect(screen.getByTestId('add-secret-modal')).toBeInTheDocument()
}

beforeEach(() => vi.clearAllMocks())

describe('scanning on a phone', () => {
  it('asks where the photo comes from before opening anything', async () => {
    await openSheet()

    const sheet = screen.getByTestId('scan-source-sheet')
    expect(sheet).toHaveTextContent('Take Photo')
    expect(sheet).toHaveTextContent('Photo Library')
    // Over the picker, not instead of it.
    expect(screen.getByTestId('add-secret-modal')).toBeInTheDocument()
    expect(calls('pick_file')).toHaveLength(0)
    expect(screen.queryByTestId('scan-source-capture')).toHaveAttribute('capture', 'environment')
  })

  it('backs out of the question to the picker', async () => {
    await openSheet()

    await userEvent.keyboard('{Escape}')

    expect(screen.queryByTestId('scan-source-sheet')).not.toBeInTheDocument()
    expect(screen.getByTestId('add-secret-modal')).toBeInTheDocument()
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
      expect(calls('pick_file')).toContainEqual({ kind: 'image', label: 'Images' })
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
})
