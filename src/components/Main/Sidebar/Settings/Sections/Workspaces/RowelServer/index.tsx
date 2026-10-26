import { useApp } from '@/store'
import Group from './Group'

/**
 * Settings › Lockboxes › Rowel Server, where the build offers it. The sign-in
 * comes back through the browser, which only the desktop listens for yet, and
 * the probe says so (`AppStatus.orgConnect`): the phone draws nothing here —
 * not a fingerprint, not a URL field — rather than a flow Rust would refuse.
 * The wide modal and the phone's pane both draw this through the section they
 * share, so the one gate serves both.
 */
export default function RowelServer() {
  const offered = useApp(state => state.status?.orgConnect ?? false)
  return offered ? <Group /> : null
}
