import type { AskTarget } from '@/api/browser'

/**
 * A target as the dialog's title names it: the site, the request pattern
 * (`host/path`, led by the methods when the agent named any), or the command
 * line itself.
 */
export const targetText = (target: AskTarget | undefined): string => {
  switch (target?.kind) {
    case 'origin':
      return target.origin
    case 'http': {
      const where = `${target.host}/${target.path.replace(/^\/+/, '')}`
      return target.methods.length ? `${target.methods.join(', ')} ${where}` : where
    }
    case 'command':
      return target.line
    default:
      return ''
  }
}
