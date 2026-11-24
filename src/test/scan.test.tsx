import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import { useUi, useVault, startEntry } from '@/store'
import {
  cleanFields,
  firstImage,
  isImagePath,
  mergeFields
} from '@/components/Main/Scan/fields'
import { runScan } from '@/components/Main/Scan/run'
import { withEntries, loginMeta, seedApp } from './utils'
import { calls, mockCommand } from './ipc'

const CARD = { number: '4242424242424242', month: '04', year: '27', name: 'ADA LOVELACE' }

const PASSPORT = {
  doc_type: 'passport',
  name: 'ANNA MARIA ERIKSSON',
  number: 'L898902C3',
  country: 'UTO',
  nationality: 'UTO',
  birth_date: '1974-08-12',
  sex: 'F',
  expiry_date: '2012-04-15',
  personal_number: 'ZE184226B'
}

const seed = () => withEntries([loginMeta({ id: 'l1', title: 'Google' })])

const scan = (path: string) => act(async () => void (await runScan(path)))

beforeEach(() => vi.clearAllMocks())

describe('scan fields', () => {
  it('takes only images, whatever the case of the extension', () => {
    expect(isImagePath('/Users/me/Desktop/card.png')).toBe(true)
    expect(isImagePath('/Users/me/IMG_0042.HEIC')).toBe(true)
    expect(isImagePath('shot.jpeg')).toBe(true)
    // What the Import drop zone is there for, and must keep getting.
    expect(isImagePath('/Users/me/export.csv')).toBe(false)
    expect(isImagePath('/Users/me/vault.swftx')).toBe(false)
    expect(isImagePath('/Users/me/scan.pdf')).toBe(false)
    // No extension at all — including a directory that only looks like one.
    expect(isImagePath('/Users/me/screenshot')).toBe(false)
    expect(isImagePath('/Users/me/photos.png/notes')).toBe(false)
  })

  it('picks the image out of a mixed drop', () => {
    expect(firstImage(['/a/readme.txt', '/a/card.jpg'])).toBe('/a/card.jpg')
    expect(firstImage(['/a/readme.txt', '/a/export.csv'])).toBeUndefined()
    expect(firstImage([])).toBeUndefined()
  })

  it('drops the fields the recognizer could not fill', () => {
    expect(cleanFields({ number: 'L898902C3', personal_number: '', sex: '   ' })).toEqual({
      number: 'L898902C3'
    })
  })

  it('fills only what is empty', () => {
    const draft = { type: 'card' as const, title: 'My card', number: '', name: 'MY OWN NAME' }

    expect(mergeFields(draft, CARD)).toEqual({
      type: 'card',
      title: 'My card',
      number: '4242424242424242',
      month: '04',
      year: '27',
      // Typed by hand, so the scan does not get to correct it.
      name: 'MY OWN NAME'
    })
  })

  it('always sets the document type', () => {
    const draft = { type: 'identity' as const, title: '', doc_type: 'passport', number: '' }

    // The default is not an answer: the document says what it is.
    expect(mergeFields(draft, { doc_type: 'id_card', number: 'X1' })).toMatchObject({
      doc_type: 'id_card',
      number: 'X1'
    })
  })

  it('leaves values that are not text alone', () => {
    const draft = { type: 'card' as const, title: '', tags: ['travel'] }

    expect(mergeFields(draft, { tags: 'ignored' }).tags).toEqual(['travel'])
  })
})

describe('scan routing', () => {
  it('starts a new entry of the kind that was recognized, seeded', async () => {
    seed()
    mockCommand('scan_image', () => ({ kind: 'card', fields: CARD }))

    await scan('/Users/me/card.png')

    expect(useVault.getState().creating).toBe('card')
    expect(useVault.getState().prefill).toEqual(CARD)
    expect(useUi.getState().scanBusy).toBe(false)
  })

  it('fills the editor already open for that kind instead of opening another', async () => {
    seed()
    startEntry('identity')
    mockCommand('scan_image', () => ({ kind: 'identity', fields: PASSPORT }))

    await scan('/Users/me/passport.jpg')

    expect(useVault.getState().creating).toBe('identity')
    expect(useVault.getState().prefill).toEqual(PASSPORT)
  })

  it('reports why nothing came back', async () => {
    seed()
    mockCommand('scan_image', () => Promise.reject({ kind: 'unrecognized', message: 'nothing recognized' }))

    await scan('/Users/me/wall.png')

    expect(useVault.getState().creating).toBeNull()
    expect(useUi.getState()).toMatchObject({ scanBusy: false, scanError: 'unreadable' })
  })
})

describe('a scanned draft', () => {
  it('opens the editor with the fields already in it', async () => {
    seed()
    seedApp({ scanSupported: true })
    mockCommand('scan_image', () => ({ kind: 'identity', fields: PASSPORT }))
    render(<Main />)

    await scan('/Users/me/passport.jpg')

    await waitFor(() =>
      expect(document.querySelector('input[name="number"]')).toHaveValue('L898902C3')
    )
    expect(document.querySelector('input[name="name"]')).toHaveValue('ANNA MARIA ERIKSSON')
    // Consumed: nothing is left to seed the next entry with.
    expect(useVault.getState().prefill).toBeNull()
  })

  it('fills the blanks of an editor that is already open', async () => {
    seed()
    seedApp({ scanSupported: true })
    startEntry('identity')
    render(<Main />)

    const name = document.querySelector('input[name="name"]') as HTMLInputElement
    await userEvent.type(name, 'MY OWN NAME')

    mockCommand('scan_image', () => ({ kind: 'identity', fields: PASSPORT }))
    await scan('/Users/me/passport.jpg')

    await waitFor(() =>
      expect(document.querySelector('input[name="number"]')).toHaveValue('L898902C3')
    )
    expect(name).toHaveValue('MY OWN NAME')
  })
})

describe('scanning from the picker', () => {
  const openFromRail = () => userEvent.click(screen.getByTestId('add-entry-button'))

  it('offers nothing where the platform cannot scan', async () => {
    seed()
    render(<Main />)
    await openFromRail()

    expect(screen.queryByTestId('add-scan-image')).not.toBeInTheDocument()
  })

  it('offers the action where it can', async () => {
    seed()
    seedApp({ scanSupported: true })
    render(<Main />)
    await openFromRail()

    const action = within(screen.getByTestId('add-secret-modal')).getByTestId('add-scan-image')
    expect(action).toHaveTextContent('Scan a card or document…')
  })

  it('routes a picked file like a drop', async () => {
    seed()
    seedApp({ scanSupported: true })
    mockCommand('pick_file', () => '/Users/me/card.png')
    mockCommand('scan_image', () => ({ kind: 'card', fields: CARD }))
    render(<Main />)
    await openFromRail()

    await userEvent.click(screen.getByTestId('add-scan-image'))

    await waitFor(() => expect(useVault.getState().creating).toBe('card'))
    expect(useUi.getState().addPicker).toBe(false)
    // The dialog is the backend's, so it is asked for by kind — that is what
    // lets it grant the path it then agrees to read (see `pickFileToRead`).
    expect(calls('pick_file')).toContainEqual({ kind: 'image', label: 'Images' })
    expect(calls('scan_image')).toContainEqual({ path: '/Users/me/card.png' })
  })

  it('leaves the picker alone when the dialog is cancelled', async () => {
    seed()
    seedApp({ scanSupported: true })
    mockCommand('pick_file', () => null)
    render(<Main />)
    await openFromRail()

    await userEvent.click(screen.getByTestId('add-scan-image'))

    expect(screen.getByTestId('add-secret-modal')).toBeInTheDocument()
    expect(calls('scan_image')).toHaveLength(0)
  })

  it('scans nothing when the dialog itself fails', async () => {
    seed()
    seedApp({ scanSupported: true })
    mockCommand('pick_file', () => Promise.reject({ kind: 'unsupported', message: 'no picker' }))
    render(<Main />)
    await openFromRail()

    await userEvent.click(screen.getByTestId('add-scan-image'))

    expect(screen.getByTestId('add-secret-modal')).toBeInTheDocument()
    expect(calls('scan_image')).toHaveLength(0)
  })

  it('keeps the tiles answering to the digits', async () => {
    seed()
    seedApp({ scanSupported: true })
    render(<Main />)
    await openFromRail()

    await userEvent.keyboard('2')

    expect(useVault.getState().creating).toBe('card')
  })

  // The phone's source sheet (see scanMobile.test) is not the desktop's way
  // in: the row goes straight to the dialog.
  it('asks no source question on the desktop', async () => {
    seed()
    seedApp({ scanSupported: true })
    render(<Main />)
    await openFromRail()

    await userEvent.click(screen.getByTestId('add-scan-image'))

    expect(screen.queryByTestId('scan-source-sheet')).not.toBeInTheDocument()
    expect(calls('pick_file')).toHaveLength(1)
  })
})
