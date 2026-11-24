import { useUi } from '@/store'

// The copy pill's shape, for a sentence rather than a fixed phrase: something
// that happened on its own and is worth a glance — a vault from the account
// arriving as a workspace. Drawn from `ui.notice`, gone when it clears.
export default function NoticeToast() {
  const notice = useUi(state => state.notice)

  if (!notice) return null

  return (
    <div
      data-testid="notice-toast"
      role="status"
      className="animate-pop fixed inset-x-safe top-4 z-50 mx-auto w-max max-w-[80vw] rounded-full bg-text px-5 py-2 text-base text-detail shadow-float"
    >
      {notice}
    </div>
  )
}
