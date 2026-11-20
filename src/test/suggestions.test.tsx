import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Show from '@/components/Main/Body/Aside/Show'
import { newEntry, setEntries, useVault } from '@/store'
import type { Entry } from '@/api/types'
import { matching } from '@/components/elements/Suggestions/match'
import { tally } from '@/utils/tally'
import { loginMeta } from './utils'
import { calls, mockCommand } from './ipc'
import { toEntryMeta } from './meta'

const input = (name: string) =>
  document.querySelector<HTMLInputElement>(`input[name="${name}"]`)!

// What the list offers, top to bottom.
const offered = () =>
  within(screen.getByRole('listbox'))
    .getAllByRole('option')
    .map(option => option.getAttribute('data-testid')?.replace('suggestion-', ''))

// A lockbox whose logins sign in with three addresses and one bare handle.
const seed = () =>
  act(() =>
    setEntries(
      ['work@acme.com', 'me@example.com', 'me@example.com', 'me@example.com', 'work@acme.com', 'octocat'].map(
        (username, index) => loginMeta({ id: `l${index}`, username })
      )
    )
  )

beforeEach(() => {
  vi.clearAllMocks()
  mockCommand('save_entry', ({ entry }) => toEntryMeta(entry as Entry))
})

describe('Suggestions on a login’s username and email', () => {
  it('offers the usernames the lockbox already holds, most used first', async () => {
    seed()
    render(<Show type="login" editing />)

    await userEvent.type(input('username'), 'c')

    expect(screen.getByRole('listbox', { name: 'Username' })).toBeInTheDocument()
    expect(offered()).toEqual(['me@example.com', 'work@acme.com', 'octocat'])
    expect(within(screen.getByTestId('suggestion-me@example.com')).getByText('3')).toBeInTheDocument()
  })

  it('waits for something to be typed', async () => {
    seed()
    render(<Show type="login" editing />)

    await userEvent.click(input('username'))
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    // Nothing to answer to, so the arrow has nothing to open either.
    await userEvent.keyboard('{ArrowDown}')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()

    await userEvent.keyboard('m')
    expect(screen.getByRole('listbox')).toBeInTheDocument()

    await userEvent.keyboard('{Backspace}')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('narrows to what answers to the typing', async () => {
    seed()
    render(<Show type="login" editing />)

    await userEvent.type(input('username'), 'ACME')
    expect(offered()).toEqual(['work@acme.com'])

    await userEvent.type(input('username'), ' nothing like it')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('offers only the usernames that are addresses to the email row', async () => {
    seed()
    render(<Show type="login" editing />)

    await userEvent.type(input('email'), 'c')

    expect(screen.getByRole('listbox', { name: 'Email' })).toBeInTheDocument()
    expect(offered()).toEqual(['me@example.com', 'work@acme.com'])
  })

  it('fills the box with the option the arrows lit, and does not save', async () => {
    seed()
    render(<Show type="login" editing />)
    const box = input('username')
    await userEvent.type(box, 'c')
    // Nothing is lit until it is reached for.
    expect(box).not.toHaveAttribute('aria-activedescendant')

    await userEvent.keyboard('{ArrowDown}{ArrowDown}')
    const lit = screen.getByTestId('suggestion-work@acme.com')
    expect(lit).toHaveAttribute('aria-selected', 'true')
    expect(box).toHaveAttribute('aria-activedescendant', lit.id)

    await userEvent.keyboard('{Enter}')
    expect(box.value).toBe('work@acme.com')
    expect(box).toHaveFocus()
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(calls('save_entry')).toHaveLength(0)
  })

  it('wraps round the ends of the list', async () => {
    seed()
    render(<Show type="login" editing />)
    await userEvent.type(input('username'), 'c')

    await userEvent.keyboard('{ArrowUp}')
    expect(screen.getByTestId('suggestion-octocat')).toHaveAttribute('aria-selected', 'true')

    await userEvent.keyboard('{ArrowDown}')
    expect(screen.getByTestId('suggestion-me@example.com')).toHaveAttribute('aria-selected', 'true')
  })

  it('fills the box with the option that is pressed, and keeps the caret in it', async () => {
    seed()
    render(<Show type="login" editing />)
    await userEvent.type(input('email'), 'me')

    await userEvent.click(screen.getByTestId('suggestion-me@example.com'))

    expect(input('email').value).toBe('me@example.com')
    expect(input('email')).toHaveFocus()
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('closes on Escape without ending the edit, and comes back on the arrow', async () => {
    seed()
    render(<Show type="login" editing />)
    newEntry('login')
    await userEvent.type(input('username'), 'me')

    await userEvent.keyboard('{Escape}')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(screen.queryByText('Discard changes?')).not.toBeInTheDocument()
    expect(useVault.getState().creating).toBe('login')
    expect(input('username').value).toBe('me')

    await userEvent.keyboard('{ArrowDown}')
    expect(screen.getByTestId('suggestion-me@example.com')).toHaveAttribute('aria-selected', 'true')
  })

  it('goes when the box is left', async () => {
    seed()
    render(<Show type="login" editing />)
    await userEvent.type(input('username'), 'c')
    expect(screen.getByRole('listbox')).toBeInTheDocument()

    await userEvent.tab()
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('still saves on ⌘⏎ with an option lit', async () => {
    seed()
    render(<Show type="login" editing />)
    await userEvent.type(input('title'), 'Acme')
    await userEvent.type(input('password'), 'hunter2')
    await userEvent.type(input('username'), 'octo')
    await userEvent.keyboard('{ArrowDown}{Meta>}{Enter}{/Meta}')

    expect(calls('save_entry')[0]).toMatchObject({ entry: { username: 'octo' } })
  })

  it('leaves the box a plain one when the lockbox has nothing to offer', async () => {
    render(<Show type="login" editing />)

    await userEvent.type(input('username'), 'c')

    expect(input('username')).not.toHaveAttribute('role')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('takes an address longer than a handle', async () => {
    render(<Show type="login" editing />)
    const address = 'firstname.lastname+newsletters@subdomain.example.co.uk'

    await userEvent.type(input('username'), address)

    expect(input('username').value).toBe(address)
  })
})

describe('matching', () => {
  const source = ['sales@acme.com', 'alex@acme.com', 'alex', 'Alex@home.org'].map(value => ({ value }))

  it('puts what starts with the typing before what only holds it', () => {
    expect(matching(source, 'al').map(option => option.value)).toEqual([
      'alex@acme.com',
      'alex',
      'Alex@home.org',
      'sales@acme.com'
    ])
  })

  it('leaves out the value already in the box', () => {
    expect(matching(source, 'alex ').map(option => option.value)).toEqual([
      'alex@acme.com',
      'Alex@home.org'
    ])
  })

  it('offers no more than a menu can show without scrolling', () => {
    const many = Array.from({ length: 20 }, (_, index) => ({ value: `user${index}` }))
    expect(matching(many, 'user')).toHaveLength(6)
  })

  it('offers nothing to an empty box', () => {
    expect(matching(source, '')).toEqual([])
    expect(matching(source, '  ')).toEqual([])
  })
})

describe('tally', () => {
  it('counts each distinct value, busiest first and alphabetical among equals', () => {
    expect(tally(['b', 'a', 'c', 'c', 'b', 'c', 'a'])).toEqual([
      ['c', 3],
      ['a', 2],
      ['b', 2]
    ])
  })
})
