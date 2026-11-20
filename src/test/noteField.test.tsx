import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { FieldsProvider, NoteField } from '@/components/elements/fields'
import type { EntryDraft } from '@/kinds/draft'
import { copy } from '@/services/copy'

vi.mock('@/services/copy', () => ({ copy: vi.fn() }))

const NOTE = 'jump host first: ssh bastion\nthen sudo -iu deploy'

const LOGIN: EntryDraft = {
  type: 'login',
  title: 'Prod console',
  website: '',
  username: 'ops',
  password: 'hunter2',
  email: '',
  otp: '',
  note: NOTE
}

const row = (set: (() => void) | null = null) =>
  render(
    <FieldsProvider value={{ entry: LOGIN, set, attempted: false }}>
      <NoteField label="Note" />
    </FieldsProvider>
  )

describe('the note row', () => {
  it('hands the whole note over from its copy button', async () => {
    row()
    await userEvent.click(screen.getByTestId('copy-note'))
    expect(copy).toHaveBeenCalledWith(NOTE, { stay: undefined })
  })

  it('leaves the text selectable, so a part of it can be taken', () => {
    row()
    expect(screen.getByTestId('entry-value-note')).toHaveClass('select-text')
  })

  it('has no copy button while the note is being edited', () => {
    row(() => {})
    expect(screen.queryByTestId('copy-note')).toBeNull()
  })
})
