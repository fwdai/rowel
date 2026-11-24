import { readEnvFile, type EnvFile } from '@/api/imports'
import type { PickedFile } from '@/api/tools'
import { isImagePath } from '@/lib/fileTypes'
import { pickedPath } from '@/lib/picked'
import { appendVars, looksLikeEnv, parseEnv, varsOf } from './parse'

/**
 * Getting a `.env` off disk and into a draft: the name a chosen file suggests
 * for the entry, and how a second file folds into a body that already has one.
 * Pure over strings, apart from `ingestEnvFile`, which is the one call to the
 * backend both the editor's drop zone and the idle-window drop make.
 */

/** `.env`, `.env.production`, `local.env` — what is named like an env file. */
export const isEnvFileName = (name: string): boolean => /^\.env(\..+)?$/i.test(name) || /\.env$/i.test(name)

/**
 * The parent folder is what the file belongs to and the suffix is which of
 * the project's environments it is: `~/code/api/.env.production` is the api's
 * production env, so `api · production`. A bare `.env` is just `api`; a
 * `local.env` names itself, `api · local`. The folder is read off the path
 * only where the path is one that ends in the file's name: an Android
 * document's address names no folder, and the title is then the name's alone.
 */
export const proposeTitle = ({ path, name }: PickedFile): string => {
  const segments = path.split(/[\\/]+/).filter(Boolean)
  const parent = segments.pop() === name ? (segments.pop() ?? '') : ''
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

/** Read a dropped or picked file into what a draft needs. Rejects as the backend does. */
export const ingestEnvFile = async (chosen: PickedFile): Promise<IngestedEnv> => {
  const file = await readEnvFile(chosen.path)
  return { ...file, title: proposeTitle(chosen) }
}

/**
 * Claim a window drop only when it belongs to env ingestion. Images are rejected
 * before the text reader so the scanner remains their sole owner; other files
 * are read once and claimed only when either their name or contents are env-like.
 */
export const ingestDroppedEnvFile = async (path: string): Promise<IngestedEnv | null> => {
  const dropped = pickedPath(path)
  if (isImagePath(dropped.name)) return null
  const file = await ingestEnvFile(dropped)
  return isEnvDrop(dropped.name, file.body) ? file : null
}
