import { readEnvFile, type EnvFile } from '@/api/imports'
import { isImagePath } from '@/lib/fileTypes'
import { appendVars, looksLikeEnv, parseEnv, varsOf } from './parse'

/**
 * Getting a `.env` off disk and into a draft: the name a dropped path suggests
 * for the entry, and how a second file folds into a body that already has one.
 * Pure over strings, apart from `ingestEnvFile`, which is the one call to the
 * backend both the editor's drop zone and the idle-window drop make.
 */

export const fileNameOf = (path: string): string => path.replace(/^.*[\\/]/, '')

/** `.env`, `.env.production`, `local.env` — what is named like an env file. */
export const isEnvFileName = (name: string): boolean => /^\.env(\..+)?$/i.test(name) || /\.env$/i.test(name)

/**
 * The parent folder is what the file belongs to and the suffix is which of
 * the project's environments it is: `~/code/api/.env.production` is the api's
 * production env, so `api · production`. A bare `.env` is just `api`; a
 * `local.env` names itself, `api · local`.
 */
export const proposeTitle = (path: string): string => {
  const segments = path.split(/[\\/]+/).filter(Boolean)
  const name = segments.pop() ?? ''
  const parent = segments.pop() ?? ''
  const env = name.startsWith('.env') ? name.slice(4).replace(/^\./, '') : name.replace(/\.env$/i, '')
  return [parent, env].filter(Boolean).join(' · ')
}

/**
 * Append the incoming vars whose key the existing body does not have, in the
 * order they arrive. The existing text is a byte-identical prefix of the result,
 * so a teammate's newer file tops yours up without touching a rotated value.
 * A key the incoming file repeats is added once, with its first value.
 */
export const mergeNewKeys = (existing: string, incoming: string): string => {
  const have = new Set(varsOf(parseEnv(existing)).map(v => v.key))
  const fresh = varsOf(parseEnv(incoming)).filter(v => {
    if (have.has(v.key)) return false
    have.add(v.key)
    return true
  })
  return appendVars(existing, fresh)
}

/**
 * Whether a file dropped on the idle window is ours: named like an env file,
 * or reading as one. Anything else belongs to whoever else is listening (the
 * Import screen's zone, the scanner) or to nobody.
 */
export const isEnvDrop = (fileName: string, body: string): boolean =>
  isEnvFileName(fileName) || looksLikeEnv(body)

export interface IngestedEnv extends EnvFile {
  /** Proposed from the path; the draft keeps it only where the title is blank. */
  title: string
}

/** Read a dropped or picked path into what a draft needs. Rejects as the backend does. */
export const ingestEnvFile = async (path: string): Promise<IngestedEnv> => {
  const file = await readEnvFile(path)
  return { ...file, title: proposeTitle(path) }
}

/**
 * Claim a window drop only when it belongs to env ingestion. Images are rejected
 * before the text reader so the scanner remains their sole owner; other files
 * are read once and claimed only when either their name or contents are env-like.
 */
export const ingestDroppedEnvFile = async (path: string): Promise<IngestedEnv | null> => {
  if (isImagePath(path)) return null
  const file = await ingestEnvFile(path)
  return isEnvDrop(fileNameOf(path), file.body) ? file : null
}
