import { describe, it, expect, vi } from 'vitest'
import { render } from '@testing-library/react'
import { onBackButtonPress } from '@tauri-apps/api/app'
import { goBack, useBack } from './useBack'
import { useBackButton } from './useBackButton'

vi.mock('@tauri-apps/api/app', () => ({ onBackButtonPress: vi.fn() }))

function Surface({ onBack, enabled }: { onBack: () => void; enabled?: boolean }) {
  useBack(onBack, enabled)
  return null
}

function Overlay({ onBack }: { onBack: () => void }) {
  useBack(onBack, true, 'overlay')
  return null
}

// A press from outside the page goes to the way back on top: the surface that
// arrived last, as a dialog opened over a screen mounts after it.
describe('useBack', () => {
  it('hands a press to the surface on top, then to the one under it', () => {
    const screen = vi.fn()
    const dialog = vi.fn()
    const { rerender } = render(
      <>
        <Surface onBack={screen} />
        <Surface onBack={dialog} />
      </>
    )

    expect(goBack()).toBe(true)
    expect(dialog).toHaveBeenCalledOnce()
    expect(screen).not.toHaveBeenCalled()

    rerender(<Surface onBack={screen} />)
    expect(goBack()).toBe(true)
    expect(screen).toHaveBeenCalledOnce()
  })

  // The read screen swapped for the held form under an open Share dialog: the
  // page that arrives second is still the one underneath.
  it('keeps a dialog above a page that remounts under it', () => {
    const read = vi.fn()
    const held = vi.fn()
    const dialog = vi.fn()
    const { rerender } = render(
      <>
        <Surface key="read" onBack={read} />
        <Overlay onBack={dialog} />
      </>
    )

    rerender(
      <>
        <Surface key="held" onBack={held} />
        <Overlay onBack={dialog} />
      </>
    )

    expect(goBack()).toBe(true)
    expect(dialog).toHaveBeenCalledOnce()
    expect(held).not.toHaveBeenCalled()

    rerender(<Surface key="held" onBack={held} />)
    expect(goBack()).toBe(true)
    expect(held).toHaveBeenCalledOnce()
    expect(read).not.toHaveBeenCalled()
  })

  it('hands a press to the overlay opened last', () => {
    const dialog = vi.fn()
    const dropdown = vi.fn()
    render(
      <>
        <Overlay onBack={dialog} />
        <Overlay onBack={dropdown} />
      </>
    )

    expect(goBack()).toBe(true)
    expect(dropdown).toHaveBeenCalledOnce()
    expect(dialog).not.toHaveBeenCalled()
  })

  it('passes over a surface that offers no way back right now', () => {
    const screen = vi.fn()
    const question = vi.fn()
    const { rerender } = render(
      <>
        <Surface onBack={screen} />
        <Surface onBack={question} enabled={false} />
      </>
    )
    goBack()
    expect(screen).toHaveBeenCalledOnce()

    rerender(
      <>
        <Surface onBack={screen} />
        <Surface onBack={question} enabled />
      </>
    )
    goBack()
    expect(question).toHaveBeenCalledOnce()
  })

  // A caller may close over its latest state (the draft's dirty flag) and hand
  // a new closure every render: the press runs the one from the last render,
  // and the surface keeps its place.
  it('runs the handler from the latest render', () => {
    const first = vi.fn()
    const latest = vi.fn()
    const { rerender } = render(<Surface onBack={first} />)
    rerender(<Surface onBack={latest} />)

    goBack()
    expect(latest).toHaveBeenCalledOnce()
    expect(first).not.toHaveBeenCalled()
  })

  it('says so when nothing on screen offers a way back', () => {
    const { unmount } = render(<Surface onBack={vi.fn()} />)
    unmount()
    expect(goBack()).toBe(false)
  })
})

function Host() {
  useBackButton()
  return null
}

describe('useBackButton', () => {
  // This suite runs as the desktop build: no listener, so nothing reaches for
  // a command that exists only on Android.
  it('listens for nothing off Android', () => {
    render(<Host />)
    expect(onBackButtonPress).not.toHaveBeenCalled()
  })
})
