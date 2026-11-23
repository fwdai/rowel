import { useApp, connectSync } from '@/store'
import ProviderList from '@/components/elements/ProviderList'

// Before a vault syncs anywhere: the choice of a provider, each with its own
// Connect. One account per vault, so this is a choice made once, and the
// account's card takes this list's place from the press on.
export default function Providers() {
  const inProgress = useApp(state => state.sync.inProgress)

  return (
    <ProviderList
      testid="settings"
      listTestid="settings-providers"
      disabled={inProgress}
      onPick={connectSync}
    />
  )
}
