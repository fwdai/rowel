import { invoke, type InvokeArgs } from '@tauri-apps/api/core'
import { isBackendError, type BackendError } from './errors'

/**
 * The single seam between the webview and Rust. Every wrapper in `@/api` goes
 * through here, and every rejection leaves as a `BackendError` — a broken IPC
 * or a thrown JS error included — so no caller has to guess what it caught.
 *
 * `args` is a record of named arguments as a rule; a `Uint8Array` travels as
 * the request's raw body instead, for the one command that takes a photo.
 */
export const call = <T>(command: string, args?: InvokeArgs): Promise<T> =>
  invoke<T>(command, args).catch((error: unknown) => {
    const normalized: BackendError = isBackendError(error)
      ? error
      : { kind: 'other', message: String(error) }
    throw normalized
  })
