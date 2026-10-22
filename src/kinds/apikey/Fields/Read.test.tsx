import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { FieldsProvider } from '@/components/elements/fields'
import type { EntryDraft } from '@/kinds/draft'
import Fields from '.'

const BODY = '4f9a0b3c7d2e1f6a8b9c0d1e2f3a4b5c'

const KEY: EntryDraft = {
  type: 'apikey',
  title: 'Coupler.io',
  apiKey: `sk_live_${BODY}`,
  environment: 'production',
  baseUrl: 'https://api.coupler.io/v1/',
  scopes: 'read:data write:data',
  expiry_date: '2027-01-01',
  note: ''
}

const read = (entry: EntryDraft = KEY) =>
  render(
    <FieldsProvider value={{ entry, set: null, attempted: false }}>
      <Fields />
    </FieldsProvider>
  )

describe('an API key, read', () => {
  it('sets the token on the card, prefix showing and the rest sealed', async () => {
    read()
    const face = screen.getByTestId('apikey-face')
    expect(face).toHaveTextContent('Secret key')
    expect(face).toHaveTextContent(`${KEY.apiKey?.length} chars`)
    const strip = screen.getByTestId('entry-value-apiKey')
    expect(strip).toHaveTextContent('sk_live_')
    expect(strip).not.toHaveTextContent(BODY)

    await userEvent.click(screen.getByTestId('reveal-apiKey'))
    expect(strip).toHaveTextContent(`sk_live_${BODY}`)

    await userEvent.click(screen.getByTestId('reveal-apiKey'))
    expect(strip).not.toHaveTextContent(BODY)
  })

  it('prints where the key is used along the foot of the card', () => {
    read()
    expect(screen.getByTestId('entry-value-environment')).toHaveTextContent('Production')
    expect(screen.getByTestId('entry-value-baseHost')).toHaveTextContent('api.coupler.io/v1')
    expect(screen.getByTestId('entry-value-expiry_date')).toHaveTextContent('2027')
  })

  it('keeps the full address and the scopes as rows under the card', () => {
    read()
    expect(screen.getByTestId('entry-value-baseUrl')).toHaveTextContent(
      'https://api.coupler.io/v1/'
    )
    const scopes = screen.getByTestId('entry-value-scopes')
    // One chip per scope; the copy hint beside them is not one.
    expect(scopes.querySelectorAll('.font-mono')).toHaveLength(2)
    expect(scopes).toHaveTextContent('read:data')
    expect(scopes).toHaveTextContent('write:data')
  })

  it('is the card alone for a key with nothing else known about it', () => {
    read({ ...KEY, environment: '', baseUrl: '', scopes: '', expiry_date: '' })
    expect(screen.getByTestId('apikey-face')).toBeInTheDocument()
    expect(screen.queryByTestId('entry-value-environment')).toBeNull()
    expect(screen.queryByTestId('entry-value-baseUrl')).toBeNull()
    expect(screen.queryByTestId('entry-value-scopes')).toBeNull()
    expect(screen.queryByText('Expires')).toBeNull()
  })

  // Only an issuer's documented stamp is printed; a format the list does not
  // know is masked whole, dashes and all.
  it('shows a key of an unknown format as dots alone until revealed', () => {
    read({ ...KEY, apiKey: 'deadbeef-1234-4abc-8def-123456789abc' })
    const strip = screen.getByTestId('entry-value-apiKey')
    expect(strip).not.toHaveTextContent('deadbeef')
    expect(strip.querySelector('[data-testid="mask"]')).not.toBeNull()
  })
})
