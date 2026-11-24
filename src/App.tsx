import { lazy, Suspense, useEffect } from 'react'
import { useApp, flowSetup, refreshApp, fileOpened } from './store'
import { takeOpenedFile, type AppStatus } from './api/app'
import { subscribeToEvents } from './store/events'
import { useBackButton } from './hooks/useBackButton'
import { useLayout } from './hooks/useLayout'
import Auth from './components/Auth'
import LockScreen from './components/Auth/LockScreen'
import UpdateToast from './components/elements/UpdateToast'
import { finishSplash } from './lib/splash'

// The lock screen is what every launch opens on, so it is the only flow that is
// worth parsing before the user has done anything. Setup and the unlocked vault
// are each reached by an explicit act (a first run, an unlock) with a frame to
// spare, and between them they are most of the app — kept out of the launch
// chunk, they no longer cost every boot.
const Start = lazy(() => import('./components/Start'))
const loadMain = () => import('./components/Main')
const Main = lazy(loadMain)

// The lock screen has had this long to paint and settle before the vault's
// chunk is fetched behind it, so an unlock swaps straight into `Main` rather
// than through the empty frame `Suspense` would otherwise show while the
// chunk arrives. Vite hands `lazy` the same module promise, so nothing loads
// twice.
const PREFETCH_MAIN_MS = 1_000

function Shell() {
  const flow = useApp(state => state.flow)
  const gate = useApp(state => state.status?.biometric)
  const compact = useLayout() === 'compact'
  switch (flow) {
    case 'setup':
      return <Start />
    case 'auth': {
      const props = { biometric: !!gate?.available, biometry: gate?.type }
      return compact ? <LockScreen {...props} /> : <Auth {...props} />
    }
    case 'main':
      return <Main />
  }
}

// Hands the screen from the splash to React. Rendered inside the flow's
// Suspense boundary, so its effect runs only once the flow root itself has
// committed — a lazy `Start` still on its way keeps the splash up rather than
// fading it off a frame `Suspense` has nothing to fill.
function SplashHandoff() {
  useEffect(finishSplash, [])
  return null
}

export default function App() {
  useBackButton()
  useEffect(() => {
    const unsubscribe = subscribeToEvents()
    // Nothing on disk is the only answer that sends us somewhere other than the
    // lock screen. `boot.ts` already routed on the probe it ran before the first
    // render; this is the same rule for a shell mounted some other way, and for
    // the launch whose probe never answered — ask once more rather than strand
    // a pristine install on a lock screen it has nothing to unlock, and stay
    // there if that fails too, since `flow` starts on it.
    const route = (status: AppStatus | null) => {
      if (status?.initialized === false) flowSetup()
    }
    const known = useApp.getState().status
    if (known) route(known)
    else void refreshApp().then(route)

    // Launched by double-clicking a backup: Rust parked the path because
    // nothing was listening yet. Asked after subscribing above, so an open
    // that lands from here on comes in as `file:opened` instead.
    takeOpenedFile()
      .then(path => {
        if (path) fileOpened(path)
      })
      .catch(() => {})

    const prefetch = setTimeout(() => void loadMain().catch(() => {}), PREFETCH_MAIN_MS)
    return () => {
      clearTimeout(prefetch)
      unsubscribe()
    }
  }, [])

  return (
    <>
      {/* Nothing to show for the frame a lazy flow takes to arrive: the splash
          is still over the app on the launch path, and a flow switch mid-session
          keeps the chrome that is already painted. */}
      <Suspense fallback={null}>
        <Shell />
        <SplashHandoff />
      </Suspense>
      <UpdateToast />
    </>
  )
}
