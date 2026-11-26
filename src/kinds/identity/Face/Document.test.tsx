import { describe, it, expect, vi } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
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
  personal_number: '9103140447',
  categories: 'AM, B1/B'
}

const show = (
  docType: DocType,
  shown = false,
  over: Record<string, string> = {},
  editing = false,
  extra: Partial<Doc> = {}
) => {
  const keys = new Set<IdentityKey>(TEMPLATES[docType].map(row => row.key))
  const doc: Doc = {
    value: key => (keys.has(key) ? ({ ...FILLED, ...over }[key] ?? '') : ''),
    set: editing ? () => {} : undefined,
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
    show('id_card', false, {}, false, { portrait: PHOTO })
    const frame = screen.getByTestId('identity-portrait')
    expect(frame.querySelector('img')).toHaveAttribute('src', PHOTO)
    // Reading, the photograph is part of the document: nothing takes it off.
    expect(screen.queryByTestId('remove-portrait')).toBeNull()
  })

  it('lets the photograph be taken off while editing', () => {
    const removePortrait = vi.fn()
    show('passport', true, {}, true, { portrait: PHOTO, removePortrait })
    fireEvent.click(screen.getByTestId('remove-portrait'))
    expect(removePortrait).toHaveBeenCalledOnce()
  })

  it('prints the number and its band once revealed', () => {
    show('passport', true)
    expect(screen.getByTestId('entry-value-number')).toHaveTextContent('X48213097')
    expect(face().textContent).toContain('P<GBRALEXANDRA')
  })

  describe('a card', () => {
    const CARDS = ['id_card', 'driver_license', 'residence_permit'] as const

    it('prints no machine-readable band: that is a passport page’s', () => {
      for (const docType of CARDS) {
        show(docType, true)
        expect(face().textContent).not.toContain('<<')
        cleanup()
      }
    })

    it('sets a licence’s categories under their number, and on no other card', () => {
      show('driver_license', true)
      expect(screen.getByTestId('entry-value-categories')).toHaveTextContent('AM, B1/B')
      cleanup()

      show('id_card', true)
      expect(screen.queryByTestId('entry-value-categories')).toBeNull()
    })

    it('masks its number until the eye is pressed', () => {
      for (const docType of CARDS) {
        show(docType)
        expect(face().textContent).not.toContain('X48213097')
        cleanup()
        show(docType, true)
        expect(screen.getByTestId('entry-value-number')).toHaveTextContent('X48213097')
        cleanup()
      }
    })

    describe('colour', () => {
      const doc = (editing = false): Doc => ({
        value: key => FILLED[key] ?? '',
        set: editing ? () => {} : undefined,
        shown: false,
        toggle: () => {}
      })

      it('is its type’s own until one is picked', () => {
        const worn = (docType: DocType) => {
          show(docType)
          const color = face().dataset.color
          cleanup()
          return color
        }
        expect(worn('driver_license')).toBe('rose')
        expect(worn('id_card')).toBe('blue')
        expect(worn('residence_permit')).toBe('green')
      })

      it('is the one the entry stores', () => {
        render(<Document docType="residence_permit" doc={doc()} color="blue" />)
        expect(face()).toHaveAttribute('data-color', 'blue')
      })

      it('falls back to the type’s own for a value that is no colour of ours', () => {
        for (const stored of ['', 'chartreuse', '__proto__']) {
          render(<Document docType="residence_permit" doc={doc()} color={stored} />)
          expect(face()).toHaveAttribute('data-color', 'green')
          cleanup()
        }
      })

      it('is picked under the card, while it is being edited', () => {
        const onColor = vi.fn()
        render(<Document docType="id_card" doc={doc(true)} onColor={onColor} />)
        // The type's own is the one lit before anything is picked.
        expect(screen.getByTestId('card-color-blue')).toBeChecked()

        fireEvent.click(screen.getByTestId('card-color-green'))
        expect(onColor).toHaveBeenCalledWith('green')
      })

      it('offers no colours while the card is read, nor on a passport', () => {
        show('id_card')
        expect(screen.queryByRole('radiogroup')).toBeNull()
        cleanup()
        render(<Document docType="passport" doc={doc(true)} onColor={() => {}} />)
        expect(screen.queryByRole('radiogroup')).toBeNull()
      })
    })

    it('says how long it has, once', () => {
      for (const docType of CARDS) {
        show(docType, false, { expiry_date: '2019-01-01' })
        expect(screen.getAllByText('Expired')).toHaveLength(1)
        cleanup()
      }
    })

    it('numbers a licence’s fields as the licence standard does', () => {
      show('driver_license', true)
      const indexed = (name: string) =>
        screen.getByTestId(`entry-value-${name}`).closest('button')!.parentElement
      expect(indexed('name')).toHaveTextContent(/^1\./)
      expect(indexed('birth_date')).toHaveTextContent(/^3\./)
      expect(indexed('issue_date')).toHaveTextContent(/^4a\./)
      expect(indexed('expiry_date')).toHaveTextContent(/^4b\./)
      expect(indexed('authority')).toHaveTextContent(/^4c\./)
      expect(indexed('number')).toHaveTextContent(/^5\./)
    })

    it('titles an ID card with the state that issued it', () => {
      show('id_card')
      expect(face()).toHaveTextContent('United Kingdom')
      expect(screen.getByTestId('identity-doc-type')).toHaveTextContent('ID card')
      expect(screen.getByTestId('identity-hologram')).toBeInTheDocument()
    })

    it('draws how far through its term a permit is', () => {
      const day = (offset: number) =>
        new Date(Date.now() + offset * 86_400_000).toISOString().slice(0, 10)
      const term = () => screen.queryByTestId('identity-term')

      show('residence_permit', false, { issue_date: day(-100), expiry_date: day(300) })
      expect(parseFloat(term()!.style.width)).toBeCloseTo(25, 0)
      cleanup()

      // Past its last day the term is spent, not overdrawn.
      show('residence_permit', false, { issue_date: day(-400), expiry_date: day(-10) })
      expect(term()!.style.width).toBe('100%')
      cleanup()

      // No term to draw until both ends are dates.
      show('residence_permit', false, { issue_date: '' })
      expect(term()).toBeNull()
    })
  })
})
