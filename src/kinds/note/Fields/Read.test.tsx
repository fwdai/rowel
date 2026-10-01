import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { FieldsProvider } from '@/components/elements/fields'
import type { EntryDraft } from '@/kinds/draft'
import Fields from '.'

const BODY = 'recovery code 8842-1907\nrecovery code 3316-5520'

const NOTE: EntryDraft = {
  type: 'note',
  title: 'Recovery codes',
  note: BODY
}

const read = (entry: EntryDraft = NOTE) =>
  render(
    <FieldsProvider value={{ entry, set: null, attempted: false }}>
      <Fields />
    </FieldsProvider>
  )

describe('a secure note, read', () => {
  it('keeps the body sealed until asked', async () => {
    read()
    const body = screen.getByTestId('entry-value-note')
    expect(body).not.toHaveTextContent('8842-1907')

    await userEvent.click(screen.getByTestId('unseal-note'))
    expect(body).toHaveTextContent('8842-1907')
    expect(body).toHaveTextContent('3316-5520')
    expect(screen.queryByTestId('unseal-note')).toBeNull()

    await userEvent.click(screen.getByTestId('reveal-note'))
    expect(body).not.toHaveTextContent('8842-1907')
  })

  it('reveals through the eye as well as the seal', async () => {
    read()
    await userEvent.click(screen.getByTestId('reveal-note'))
    expect(screen.getByTestId('entry-value-note')).toHaveTextContent('8842-1907')
  })

  it('reads custom fields under the body, and has no section without them', () => {
    const { unmount } = read()
    expect(screen.queryByText('Custom fields')).toBeNull()
    unmount()

    read({ ...NOTE, extra: [{ label: 'Issued by', value: 'GitHub' }] })
    expect(screen.getByText('Custom fields')).toBeInTheDocument()
    expect(screen.getByTestId('entry-extra-label-0')).toHaveTextContent('Issued by')
    expect(screen.getByTestId('entry-extra-value-0')).toHaveTextContent('GitHub')
  })

  it('keeps the editor unsealed', () => {
    render(
      <FieldsProvider value={{ entry: NOTE, set: () => {}, attempted: false }}>
        <Fields />
      </FieldsProvider>
    )
    expect(screen.getByRole('textbox')).toHaveValue(BODY)
    expect(screen.queryByTestId('unseal-note')).toBeNull()
  })
})
