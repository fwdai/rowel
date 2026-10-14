import { invoke, type InvokeArgs, type InvokeOptions } from '@tauri-apps/api/core'
import { isBackendError, type BackendError } from './errors'

/**
 * The single seam between the webview and Rust. Every wrapper in `@/api` goes
 * through here, and every rejection leaves as a `BackendError` — a broken IPC
 * or a thrown JS error included — so no caller has to guess what it caught.
 *
 * `args` is a record of named arguments as a rule; a `Uint8Array` travels as
 * the request's raw body instead, for the commands that take a file, and
 * whatever else such a command needs rides as `options.headers`. Once Tauri
 * has fallen back from its custom-protocol IPC to `postMessage` — on iOS, a
 * request that failed while the camera had the webview off screen — the same
 * bytes arrive as a JSON array of numbers instead, and Rust takes either
 * (`commands::body_bytes`).
 */
export const call = <T>(command: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> =>
  invoke<T>(command, args, options).catch((error: unknown) => {
    const normalized: BackendError = isBackendError(error)
      ? error
      : { kind: 'other', message: String(error) }
    throw normalized
  })
