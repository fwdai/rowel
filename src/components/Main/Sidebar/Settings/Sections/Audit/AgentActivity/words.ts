import type { TFunction } from 'i18next'
import type { ActorKind, AgentEvent, ChainBreak, WriterChain } from '@/api/browser'
import { CLIENT_KIND_LABEL } from '@/lib/clientPlatform'

/**
 * The log's vocabulary in plain words. Rust writes the row with the reason
 * the broker had (`no_rule`, `timeout`); the person reading Vault Health gets
 * the sentence behind it. A reason this list does not know is shown as Rust
 * spelled it rather than dropped — a new reason is still a reason.
 */
export const actorWords = (t: TFunction, kind: ActorKind): string => {
  switch (kind) {
    case 'human':
      return t('You')
    case 'policy':
      return t('Policy')
    case 'server':
      return t('Server')
    default:
      return t(CLIENT_KIND_LABEL[kind])
  }
}

const denialWords = (t: TFunction, reason: string): string => {
  switch (reason) {
    case 'no_target':
      return t('nowhere it could be used')
    case 'lockbox':
      return t('the lockbox does not allow agents')
    case 'no_rule':
      return t('no rule allowed it')
    case 'no_candidates':
      return t('no item matched')
    case 'item':
      return t('the item was not allowed')
    case 'user':
      return t('you refused')
    case 'timeout':
      return t('the ask timed out')
    default:
      return reason
  }
}

export const outcomeWords = (t: TFunction, event: AgentEvent): string => {
  switch (event.event) {
    case 'used':
      return t('Used')
    case 'exposed':
      return t('Exposed')
    case 'denied':
      return event.reason
        ? t('Denied: {{reason}}', { reason: denialWords(t, event.reason) })
        : t('Denied')
  }
}

const breakWords = (t: TFunction, reason: ChainBreak): string => {
  switch (reason) {
    case 'writer':
      return t('a row from another device')
    case 'fork':
      return t('two rows with the same number')
    case 'gap':
      return t('a missing row')
    case 'prevHash':
      return t('a row that does not follow the one before it')
    case 'hash':
      return t('a row that was altered')
  }
}

/** A writer named the way the person at this device knows it: theirs, or another's by the start of its id. */
export const writerWords = (t: TFunction, writer: Pick<WriterChain, 'writerId' | 'thisDevice'>) =>
  writer.thisDevice ? t('this device') : t('device {{id}}', { id: writer.writerId.slice(0, 8) })

/** The verification in one line: the whole log holds, or where it first stops holding. */
export const verificationWords = (t: TFunction, writers: WriterChain[]): string => {
  const broken = writers.find(writer => writer.break !== null)
  if (broken?.break) {
    return t("Break in {{who}}'s log at row {{seq}}: {{reason}}", {
      who: writerWords(t, broken),
      seq: broken.break.seq,
      reason: breakWords(t, broken.break.reason)
    })
  }
  const rows = writers.reduce((sum, writer) => sum + writer.rows, 0)
  return t('Log verified: {{rows}} from {{devices}}', {
    rows: t('{{count}} rows', { count: rows }),
    devices: t('{{count}} devices', { count: writers.length })
  })
}
