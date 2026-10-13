import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Settings from '@/components/Main/Sidebar/Settings'
import Modal from '@/components/elements/Modal'
import Sheet from '@/components/elements/Sheet'

const openSettings = async () => {
  await userEvent.click(screen.getByTestId('settings-button'))
  return screen.getByTestId('settings-modal')
}

describe('Modal close control', () => {
  // The default card's close is a bare glyph; without a name every dialog that
  // inherits it announces as "button".
  it('names the close button', () => {
    render(
      <Modal onClose={vi.fn()} testid="plain">
        <p>body</p>
      </Modal>
    )
    expect(screen.getByTestId('modal-close')).toHaveAccessibleName('Close')
  })
})

describe('Modal focus management', () => {
  it('moves focus inside the card when it opens', async () => {
    render(<Settings />)
    const modal = await openSettings()

    expect(modal.contains(document.activeElement)).toBe(true)
  })

  // A control the stylesheet hides for this shell cannot take focus; starting
  // there would leave the keyboard behind the card. jsdom reports nothing
  // about visibility, so the engine's answer is stood in for here.
  it('starts focus on the first control that is actually shown', () => {
    Object.defineProperty(HTMLElement.prototype, 'checkVisibility', {
      configurable: true,
      value(this: HTMLElement) {
        return !this.hasAttribute('data-hidden')
      }
    })
    try {
      render(
        <Modal onClose={vi.fn()} testid="plain" hideClose>
          <button type="button" data-hidden="">
            hidden
          </button>
          <button type="button">shown</button>
        </Modal>
      )
      expect(document.activeElement).toBe(screen.getByText('shown'))
    } finally {
      delete (HTMLElement.prototype as { checkVisibility?: unknown }).checkVisibility
    }
  })

  it('keeps Tab from leaving the card', async () => {
    render(<Settings />)
    const modal = await openSettings()

    const tabbables = modal.querySelectorAll<HTMLElement>('button, input, select')
    tabbables[tabbables.length - 1].focus()
    await userEvent.tab()

    expect(modal.contains(document.activeElement)).toBe(true)
  })

  it('returns focus to the trigger on close', async () => {
    render(<Settings />)
    const trigger = screen.getByTestId('settings-button')
    await openSettings()
    // Park focus deep inside the card, so passing can only mean it was restored.
    screen.getByTestId('settings-nav-security').focus()

    await userEvent.keyboard('{Escape}')

    expect(trigger).toHaveFocus()
  })

  it('leaves the underlying dialog alone when a second one is on top', async () => {
    const onClose = vi.fn()
    render(
      <>
        <Modal onClose={onClose} testid="under">
          <button type="button">under</button>
        </Modal>
        <Modal onClose={vi.fn()} testid="over">
          <button type="button">over</button>
        </Modal>
      </>
    )

    await userEvent.keyboard('{Escape}')

    expect(onClose).not.toHaveBeenCalled()
  })
})

// The compact frame is a dialog too, so it owes the keyboard the same contract.
describe('Sheet focus management', () => {
  const renderSheet = (onClose = vi.fn()) => {
    const result = render(
      <Sheet onClose={onClose} testid="sheet">
        <input aria-label="first" />
        <button type="button">last</button>
      </Sheet>
    )
    return { ...result, sheet: screen.getByTestId('sheet') }
  }

  it('moves focus inside when it opens', () => {
    const { sheet } = renderSheet()
    expect(sheet.contains(document.activeElement)).toBe(true)
  })

  it('keeps Tab from leaving the sheet', async () => {
    const { sheet } = renderSheet()
    screen.getByText('last').focus()
    await userEvent.tab()
    expect(sheet.contains(document.activeElement)).toBe(true)
  })

  it('closes on Escape and returns focus to the trigger', async () => {
    const trigger = document.createElement('button')
    document.body.appendChild(trigger)
    trigger.focus()
    const onClose = vi.fn()
    const { unmount } = renderSheet(onClose)
    expect(trigger).not.toHaveFocus()

    await userEvent.keyboard('{Escape}')
    expect(onClose).toHaveBeenCalledOnce()

    unmount()
    expect(trigger).toHaveFocus()
    trigger.remove()
  })
})
