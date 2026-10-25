import type { ClientKind } from '@/api/browser'
import type { TKey } from '@/i18n'

/**
 * The platforms an agent may say it runs on (`hello.platform`), by the name
 * their makers spell them. Product names are never translated; a platform not
 * named here is shown as the agent spelled it, which is what the user will
 * recognise.
 */
const PLATFORMS: Record<string, string> = {
  'claude-code': 'Claude Code',
  codex: 'Codex',
  cursor: 'Cursor',
  'browser-use': 'Browser Use',
  cli: 'Rowel CLI'
}

export const platformLabel = (platform: string | undefined): string | undefined =>
  platform ? (PLATFORMS[platform] ?? platform) : undefined

/** What to call a client of each kind, untranslated — call sites wrap it in `t()`. */
export const CLIENT_KIND_LABEL: Record<ClientKind, TKey> = {
  extension: 'Browser extension',
  agent: 'Agent',
  sidecar: 'Command line tool'
}
