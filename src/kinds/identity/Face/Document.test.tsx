import { describe, it, expect, vi } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { DOC_TYPES, TEMPLATES, type DocType, type IdentityKey } from '../templates'
import type { Doc } from './DocCell'
import Document from './Document'

// A document holding every field, filtered by its own template exactly as the
// face's own reader does — so a type never prints a value it has no row for.
const FILLED: Record<string, string> = {
  name: 'Alexandra Marie Reinholt',
  number: 'X48213097',
  country: 'GBR',
  nationality: 'GBR',
  birth_date: '1991-03-14',
  sex: 'F',
  issue_date: '2021-06-02',
  expiry_date: '2031-06-01',
  authority: 'HMPO',
  personal_number: '9103140447'
}

const show = (
  docType: DocType,
  shown = false,
  over: Record<string, string> = {},
  extra: Partial<Doc> = {}
) => {
  const keys = new Set<IdentityKey>(TEMPLATES[docType].map(row => row.key))
  const doc: Doc = {
    value: key => (keys.has(key) ? ({ ...FILLED, ...over }[key] ?? '') : ''),
    shown,
    toggle: () => {},
    ...extra
  }
  render(<Document docType={docType} doc={doc} />)
}

const PHOTO = 'data:image/jpeg;base64,/9j/4AAQSkZJRg=='

const face = () => screen.getByTestId('identity-face')

describe('the document face', () => {
  it('prints every kind of document on the same paper', () => {
    for (const docType of DOC_TYPES) {
      show(docType)
      expect(face()).toHaveAttribute('data-doc-type', docType)
      expect(screen.getByTestId('entry-value-name')).toHaveTextContent('Alexandra')
      cleanup()
    }
  })

  /*
   * The one part of the geometry a unit test can hold: that each document asks
   * to be its real shape. jsdom implements no layout — every `offsetHeight` here
   * is 0 — so whether the printing then *fits* that shape is not observable in
   * this suite, and clipping or overlap would have to be caught in a browser.
   * What this does catch is a document being handed the wrong format.
   */
  it('asks to be the shape the real document is', () => {
    const ratio = (docType: DocType) => {
      show(docType)
      const spacer = face().querySelector<HTMLElement>('[style*="aspect-ratio"]')!
      const [w, h] = spacer.style.aspectRatio.split('/').map(Number)
      cleanup()
      return w / h
    }

    // ID-3, the passport data page: 125 × 88mm, and squarer than a card.
    expect(ratio('passport')).toBeCloseTo(1.42, 2)
    // ID-1, the wallet card the other four share: 85.6 × 53.98mm.
    for (const docType of DOC_TYPES.filter(type => type !== 'passport')) {
      expect(ratio(docType)).toBeCloseTo(1.586, 2)
    }
    expect(ratio('passport')).toBeLessThan(ratio('id_card'))
  })

  it('prints no row the document has no place for', () => {
    // A licence template carries no nationality; the value is there to be read
    // and the face still has to leave it off.
    show('driver_license')
    expect(screen.queryByTestId('entry-value-nationality')).toBeNull()
    expect(screen.getByTestId('entry-value-number')).toBeInTheDocument()
  })

  it('says how long the document has, once', () => {
    show('passport', false, { expiry_date: '2019-01-01' })
    expect(screen.getAllByText('Expired')).toHaveLength(1)
  })

  it('says nothing about validity where there is no expiry', () => {
    show('other', false, { expiry_date: '' })
    expect(screen.queryByText('Expired')).toBeNull()
  })

  it('keeps the secrets masked until the eye is pressed', () => {
    show('passport')
    expect(screen.getByTestId('entry-value-number')).not.toHaveTextContent('X48213097')
    // The band is generated from the same values and must not undo the mask.
    expect(face().textContent).not.toContain('X48213097')
    expect(face().textContent).not.toContain('9103140447')
  })

  it('prints a silhouette where the document has no photograph', () => {
    show('passport')
    const frame = screen.getByTestId('identity-portrait')
    expect(frame.querySelector('img')).toBeNull()
    expect(frame.querySelector('svg')).not.toBeNull()
  })

  it("prints the holder's photograph where it has one", () => {
    show('id_card', false, {}, { portrait: PHOTO })
    const frame = screen.getByTestId('identity-portrait')
    expect(frame.querySelector('img')).toHaveAttribute('src', PHOTO)
    // Reading, the photograph is part of the document: nothing takes it off.
    expect(screen.queryByTestId('remove-portrait')).toBeNull()
  })

  it('lets the photograph be taken off while editing', async () => {
    const removePortrait = vi.fn()
    show('passport', true, {}, { portrait: PHOTO, set: () => {}, removePortrait })
    await userEvent.click(screen.getByTestId('remove-portrait'))
    expect(removePortrait).toHaveBeenCalledOnce()
  })

  it('prints the number and its band once revealed', () => {
    show('passport', true)
    expect(screen.getByTestId('entry-value-number')).toHaveTextContent('X48213097')
    expect(face().textContent).toContain('P<GBRALEXANDRA')
  })
})
