import { useUi } from '@/store'
import PasswordChangedDialog from './PasswordChangedDialog'

/**
 * The prompt for the master password another device changed this vault to.
 * Mounted once, here in the shell's overlays, so every place that shows the
 * failure — Settings › Lockboxes, the header's sync chip — raises the same
 * dialog (`openPasswordPrompt`) rather than drawing one of its own. A lock
 * drops the flag with the rest of the session's surfaces.
 */
export default function PasswordChanged() {
  const open = useUi(state => state.passwordPrompt)
  return open ? <PasswordChangedDialog /> : null
}
