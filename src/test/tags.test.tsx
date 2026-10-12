import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Sidebar from '@/components/Main/Sidebar'
import ListColumn from '@/components/Main/Body/ListColumn'
import { useUi, setView, setFilterTag } from '@/store'
import { withEntries, loginMeta } from './utils'

const entries = [
  loginMeta({ id: 'a', title: 'Google', tags: ['work', 'mail'] }),
  loginMeta({ id: 'b', title: 'Airbnb', tags: ['work'], favorite: true }),
  loginMeta({ id: 'c', title: 'Monzo', tags: ['money'] }),
  loginMeta({ id: 'd', type: 'card', title: 'Visa', tags: ['money'], urlHost: '' })
]

const titles = () => screen.getAllByTestId('entry-item-title').map(el => el.textContent)
// The tags are a pick-one menu, so each row is a `menuitemradio`.
const options = () => screen.getAllByRole('menuitemradio').map(item => item.textContent)

// The rail and the column it drives: Tags is a view, so it is read across the
// tile that lights, the title, and what the column lists.
const seed = (rows = entries, prepare?: () => void) => {
  withEntries(rows)
  prepare?.()
  return render(
    <>
      <Sidebar />
      <ListColumn />
    </>
  )
}

const open = () => userEvent.click(screen.getByTestId('tags-button'))
const pick = (tag: string) => userEvent.click(screen.getByTestId(`tag-option-${tag}`))

beforeEach(() => vi.clearAllMocks())

describe('the tags menu', () => {
  it('lists every tag in the vault, busiest first, with its count', async () => {
    seed()
    await open()

    expect(options()).toEqual(['money2', 'work2', 'mail1'])
    // Opening the menu is not yet navigating.
    expect(useUi.getState().view).toBe('items')
    expect(screen.getByTestId('tags-button')).toHaveAttribute('aria-pressed', 'false')
  })

  it('counts the whole vault whichever view it is opened from', async () => {
    seed(entries, () => setView('favorites'))
    await open()

    expect(options()).toEqual(['money2', 'work2', 'mail1'])
  })

  it('says how to fill itself when the vault carries no tags', async () => {
    seed([loginMeta({ id: 'bare', title: 'Basecamp' })])
    await open()

    expect(screen.getByTestId('tags-empty')).toBeInTheDocument()
    expect(screen.getByText('Add tags to an entry and they show up here.')).toBeInTheDocument()
    expect(screen.queryAllByRole('menuitemradio')).toHaveLength(0)
  })

  it('closes on Escape and on a click outside', async () => {
    seed()

    await open()
    await userEvent.keyboard('{Escape}')
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()

    await open()
    await userEvent.click(screen.getByTestId('dropdown-scrim'))
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
  })
})

describe('the tags view', () => {
  it('opens on the picked tag, lit in the rail in place of the view it was picked from', async () => {
    seed(entries, () => setView('favorites'))
    await open()
    await pick('work')

    expect(useUi.getState().view).toBe('tags')
    expect(useUi.getState().filterTag).toBe('work')
    expect(screen.getByTestId('tags-button')).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByTestId('view-favorites')).toHaveAttribute('aria-pressed', 'false')
    expect(screen.getByTestId('view-items')).toHaveAttribute('aria-pressed', 'false')
    // The tag as the entry wears it: no `#`, a tag is not a hashtag.
    expect(screen.getByTestId('list-title')).toHaveTextContent('work')
    expect(screen.getByTestId('active-tag')).toHaveTextContent('work')
    expect(screen.getByTestId('list-title')).not.toHaveTextContent('#')
    // Picking is also what closes the menu.
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
  })

  it('shows the items carrying the tag from across the vault', async () => {
    seed(entries, () => setView('favorites'))
    await open()
    await pick('work')

    // The starred and the unstarred row both: a tag is not scoped to a view.
    expect(titles()).toEqual(expect.arrayContaining(['Google', 'Airbnb']))
    expect(titles()).toHaveLength(2)
  })

  it('marks the open tag in the menu and switches to another', async () => {
    seed()
    await open()
    await pick('work')

    await open()
    expect(screen.getByTestId('tag-option-work')).toContainElement(
      screen.getByTestId('tag-option-work').querySelector('svg')
    )
    await pick('money')

    expect(useUi.getState().filterTag).toBe('money')
    expect(titles()).toEqual(expect.arrayContaining(['Monzo', 'Visa']))
    expect(titles()).toHaveLength(2)
  })

  it('leaves for All Items from the chip', async () => {
    seed()
    await open()
    await pick('work')
    await userEvent.click(screen.getByTestId('active-tag'))

    expect(useUi.getState().view).toBe('items')
    expect(useUi.getState().filterTag).toBeNull()
    expect(screen.queryByTestId('active-tag')).not.toBeInTheDocument()
    expect(titles()).toHaveLength(4)
  })

  it('drops the tag on the way to another view', async () => {
    seed()
    await open()
    await pick('work')
    await userEvent.click(screen.getByTestId('view-favorites'))

    expect(useUi.getState().filterTag).toBeNull()
    expect(screen.getByTestId('tags-button')).toHaveAttribute('aria-pressed', 'false')
    expect(titles()).toEqual(['Airbnb'])
  })

  it('does not narrow the other views', () => {
    seed(entries, () => {
      setView('favorites')
      setFilterTag('money')
    })

    // Favorites is the starred rows, whatever tag was left in the store.
    expect(titles()).toEqual(['Airbnb'])
    expect(screen.queryByTestId('active-tag')).not.toBeInTheDocument()
  })

  it('names the tag in the title', async () => {
    seed()
    await open()
    await pick('money')

    expect(screen.getByTestId('list-title')).toHaveTextContent('money')
    expect(screen.getByTestId('active-tag')).not.toHaveTextContent('#')
  })
})
