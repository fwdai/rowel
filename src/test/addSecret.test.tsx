import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import { useUi, useVault, setView } from '@/store'
import { withEntries, loginMeta } from './utils'

const seed = () => withEntries([loginMeta({ id: 'l1', title: 'Google' })])

const openFromRail = () => userEvent.click(screen.getByTestId('add-entry-button'))
const modal = () => screen.queryByTestId('add-secret-modal')

beforeEach(() => vi.clearAllMocks())

describe('add a secret', () => {
  it('opens the picker from the rail button', async () => {
    seed()
    render(<Main />)
    expect(modal()).not.toBeInTheDocument()

    await openFromRail()

    const dialog = screen.getByTestId('add-secret-modal')
    expect(dialog).toHaveAttribute('role', 'dialog')
    // The dialog is named by its own title, not a duplicated aria-label.
    expect(dialog).toHaveAccessibleName('Add a secret')
    // The rail button no longer starts an entry by itself.
    expect(useVault.getState().creating).toBeNull()
  })

  it('offers one tile per kind', async () => {
    seed()
    render(<Main />)
    await openFromRail()

    const dialog = within(screen.getByTestId('add-secret-modal'))
    expect(dialog.getByTestId('add-kind-login')).toHaveTextContent('Login')
    expect(dialog.getByTestId('add-kind-card')).toHaveTextContent('Credit card')
    expect(dialog.getByTestId('add-kind-note')).toHaveTextContent('Secure note')
    expect(dialog.getByTestId('add-kind-identity')).toHaveTextContent('Identity')
    expect(dialog.getByTestId('add-kind-ssh')).toHaveTextContent('SSH key')
    expect(dialog.getByTestId('add-kind-apikey')).toHaveTextContent('API key')
    expect(dialog.getByTestId('add-kind-env')).toHaveTextContent('Env file')
    // Three to a row leaves the kind's one-line description to the tooltip.
    expect(dialog.getByTestId('add-kind-login')).toHaveAttribute(
      'title',
      'Passwords for apps & sites'
    )
    expect(dialog.getByTestId('add-kind-login')).toHaveAccessibleName('Login')
  })

  it('starts an entry of the chosen kind and closes', async () => {
    seed()
    render(<Main />)
    await openFromRail()

    await userEvent.click(screen.getByTestId('add-kind-card'))

    expect(useVault.getState().creating).toBe('card')
    expect(useUi.getState().addPicker).toBe(false)
    expect(modal()).not.toBeInTheDocument()
  })

  it('focuses the first tile and moves focus with the arrow keys', async () => {
    seed()
    render(<Main />)
    await openFromRail()

    expect(screen.getByTestId('add-kind-login')).toHaveFocus()
    await userEvent.keyboard('{ArrowRight}')
    expect(screen.getByTestId('add-kind-card')).toHaveFocus()
    // Three columns: down from the second tile lands on the fifth.
    await userEvent.keyboard('{ArrowDown}')
    expect(screen.getByTestId('add-kind-ssh')).toHaveFocus()
    // Left, the fourth.
    await userEvent.keyboard('{ArrowLeft}')
    expect(screen.getByTestId('add-kind-identity')).toHaveFocus()
    // Down again lands on the last tile, the seventh, alone on its row.
    await userEvent.keyboard('{ArrowDown}')
    expect(screen.getByTestId('add-kind-env')).toHaveFocus()
    // Past the last tile, the move wraps around to the first.
    await userEvent.keyboard('{ArrowRight}')
    expect(screen.getByTestId('add-kind-login')).toHaveFocus()
  })

  it('picks the nth kind by digit', async () => {
    seed()
    render(<Main />)
    await openFromRail()

    await userEvent.keyboard('3')

    expect(useVault.getState().creating).toBe('note')
    expect(modal()).not.toBeInTheDocument()
  })

  it('closes on Escape without starting anything', async () => {
    seed()
    render(<Main />)
    await openFromRail()

    await userEvent.keyboard('{Escape}')

    expect(modal()).not.toBeInTheDocument()
    expect(useVault.getState().creating).toBeNull()
  })

  it('opens on ⌘N', async () => {
    seed()
    render(<Main />)

    await userEvent.keyboard('{Meta>}n{/Meta}')

    expect(screen.getByTestId('add-secret-modal')).toBeInTheDocument()
  })

  it('stays where it was when the picker is dismissed', async () => {
    seed()
    render(<Main />)
    setView('archive')

    await openFromRail()
    await userEvent.keyboard('{Escape}')

    // Only committing to a kind leaves the view; asking does not.
    expect(useUi.getState().view).toBe('archive')
  })

  it('leaves the health view so the new form has a list to land in', async () => {
    seed()
    render(<Main />)
    setView('health')

    await openFromRail()
    await userEvent.click(screen.getByTestId('add-kind-login'))

    expect(useUi.getState().view).toBe('items')
    expect(useVault.getState().creating).toBe('login')
  })

  it('is reachable from the empty detail pane', async () => {
    withEntries([])
    render(<Main />)

    const button = screen.getByTestId('create-first-entry-button')
    expect(button).toHaveTextContent('Add a secret')

    await userEvent.click(button)
    expect(screen.getByTestId('add-secret-modal')).toBeInTheDocument()
  })

  describe('from the palette', () => {
    const openPalette = () => userEvent.keyboard('{Meta>}k{/Meta}')

    it('lists a command per kind', async () => {
      seed()
    render(<Main />)
      await openPalette()

      const palette = within(screen.getByTestId('command-palette'))
      // Each kind command is named the same way the editor sheet and the empty
      // panes name it, from the kind registry.
      expect(palette.getByText('Add a login')).toBeInTheDocument()
      expect(palette.getByText('Add a credit card')).toBeInTheDocument()
      expect(palette.getByText('Add a secure note')).toBeInTheDocument()
      expect(palette.getByText('Add a secret')).toBeInTheDocument()
    })

    it('starts the entry directly, without the picker', async () => {
      seed()
    render(<Main />)
      await openPalette()

      // "Add a secure note" also names an empty-state action, so scope it.
      const palette = within(screen.getByTestId('command-palette'))
      await userEvent.click(palette.getByText('Add a secure note'))

      expect(useVault.getState().creating).toBe('note')
      expect(useUi.getState().addPicker).toBe(false)
      expect(modal()).not.toBeInTheDocument()
    })

    it('opens the picker from "Add a secret"', async () => {
      seed()
    render(<Main />)
      await openPalette()

      // The rail tooltip carries the same words, so scope to the palette.
      const palette = within(screen.getByTestId('command-palette'))
      await userEvent.click(palette.getByText('Add a secret'))

      expect(screen.getByTestId('add-secret-modal')).toBeInTheDocument()
      expect(screen.queryByTestId('command-palette')).not.toBeInTheDocument()
      expect(useVault.getState().creating).toBeNull()
    })
  })
})
