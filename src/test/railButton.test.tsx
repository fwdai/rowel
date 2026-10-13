import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import RailButton from '@/components/elements/RailButton'

describe('RailButton', () => {
  it('is a button named by its label, with the label as its tooltip', async () => {
    render(
      <RailButton label="All Items" testid="rail">
        <span />
      </RailButton>,
    )

    const button = screen.getByRole('button', { name: 'All Items' })
    expect(button).toHaveAttribute('type', 'button')
    expect(button).not.toHaveAttribute('aria-pressed')
    expect(screen.getByTestId('rail')).toBe(button)
    // The panel only exists while shown.
    expect(screen.queryByText('All Items')).not.toBeInTheDocument()

    // Hovering shows the same copy — visible, but kept out of the AT tree so
    // it does not double the button's own name.
    await userEvent.hover(button)
    const tooltip = await screen.findByText('All Items')
    expect(tooltip).toHaveAttribute('aria-hidden', 'true')
    expect(tooltip).toHaveAttribute('role', 'tooltip')

    await userEvent.unhover(button)
    expect(screen.queryByText('All Items')).not.toBeInTheDocument()
  })

  it('reflects the selected state', () => {
    const { rerender } = render(
      <RailButton label="All Items" selected={false}>
        <span />
      </RailButton>,
    )
    expect(screen.getByRole('button')).toHaveAttribute('aria-pressed', 'false')

    rerender(
      <RailButton label="All Items" selected>
        <span />
      </RailButton>,
    )
    expect(screen.getByRole('button')).toHaveAttribute('aria-pressed', 'true')
  })

  it('fires on click', async () => {
    const onClick = vi.fn()
    render(
      <RailButton label="Settings" testid="rail" onClick={onClick}>
        <span />
      </RailButton>,
    )

    await userEvent.click(screen.getByTestId('rail'))
    expect(onClick).toHaveBeenCalledTimes(1)
  })
})
