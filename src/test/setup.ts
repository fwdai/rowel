import '@testing-library/jest-dom/vitest'
import { beforeEach, vi } from 'vitest'
import { resetEvents } from './events'
import { resetIpc } from './ipc'
import { setLayout } from './layout'
import { resetStores } from './utils'

// jsdom implements no layout, so it ships no scrollIntoView.
Element.prototype.scrollIntoView = vi.fn()

// jsdom has no object URLs; a view that points the DOM at a blob (an
// attachment's thumbnail or preview) gets a distinct fake per call.
let objectUrls = 0
URL.createObjectURL = () => `blob:test/${++objectUrls}`
URL.revokeObjectURL = () => {}

// Every suite starts on the wide shell — the one the desktop window and all the
// pre-existing tests assume — with pristine stores and preferences. A compact
// test calls `setLayout('compact')` itself.
setLayout('wide')
beforeEach(() => {
  setLayout('wide')
  resetStores()
})

// The Rust backend is built in parallel, so the one seam that reaches it is
// faked here and every command answers out of `./ipc`. A spec overrides the
// single call it is about with `mockCommand`.
vi.mock('@tauri-apps/api/core', async () => {
  const { invokeMock } = await import('./ipc')
  return { invoke: invokeMock }
})

beforeEach(resetIpc)

// The backend's event stream, faked as a bus (see `./events`): a subscription
// from a previous test would otherwise still be on it, so it is emptied with
// everything else — before the per-file hooks that subscribe.
beforeEach(resetEvents)

vi.mock('@/api/events', async orig => {
  const { onMock } = await import('./events')
  return { ...(await orig<typeof import('@/api/events')>()), on: onMock }
})

vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: vi.fn().mockResolvedValue(undefined)
}))

// The app store imports the updater at module load.
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }))
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: vi.fn() }))

// The window's focus stream `useObscured` subscribes to, plus the focus state it
// reconciles its first guess against — jsdom reports the document unfocused, so
// without `isFocused` every suite would render behind the privacy cover. Focus
// never changes by default, so the cover stays off; a suite about it replaces
// this with a mock that keeps the handlers (see PrivacyScreen/index.test.tsx).
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onFocusChanged: vi.fn().mockResolvedValue(() => {}),
    isFocused: vi.fn().mockResolvedValue(true)
  })
}))

// The drag-drop stream `useFileDrop` subscribes to. Nothing drops a file by
// default; a suite that needs to replaces this with a mock that keeps the
// handlers (see envIngest.test.tsx).
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: vi.fn().mockResolvedValue(() => {})
  })
}))

// Components under test call useTranslation(); the singleton must be
// initialized once before any of them render.
const { initI18n } = await import('@/i18n')
await initI18n('en-US')
