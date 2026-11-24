import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { pickFileToRead } from '@/api/tools'
import { describeError } from '@/api/errors'
import { stampSession } from '@/store'
import { ingestDroppedEnvFile, ingestEnvFile, type IngestedEnv } from './ingest'

/**
 * The lifecycle shared by every interactive env-file entry point.
 *
 * Only the newest request may publish a result, completion calls the latest
 * consumer (rather than the render that began the read), and failures become
 * visible state. Unmount invalidates the request so a departed surface cannot
 * update itself later, and a lock does too: a file picked or read for the
 * session that is gone is not handed to the one open now (see `stampSession`).
 */
export function useEnvIngest(onFile: (file: IngestedEnv) => void) {
  const consumer = useRef(onFile)
  const request = useRef(0)
  const [error, setError] = useState<string | null>(null)

  // Match the committed render before any in-flight read is allowed to publish.
  // (Writing a ref during render is deliberately forbidden by this repo's hook
  // lint rules; the same indirection is used by `useFileDrop`.)
  useLayoutEffect(() => {
    consumer.current = onFile
  })

  useEffect(
    () => () => {
      ++request.current
    },
    []
  )

  // `load` is handed the same check, for a step that should not start once it
  // fails: reading a file picked for a session that has since locked.
  const run = async (load: (live: () => boolean) => Promise<IngestedEnv | null>) => {
    const mine = ++request.current
    const current = stampSession()
    const live = () => mine === request.current && current()
    setError(null)
    try {
      const file = await load(live)
      if (live() && file) consumer.current(file)
    } catch (cause) {
      if (live()) setError(describeError(cause))
    }
  }

  const drop = (path: string) => run(() => ingestDroppedEnvFile(path))
  const pick = () =>
    run(async live => {
      // Any file may be a .env, so the dialog filters nothing; it runs in Rust
      // so that the read that follows has a path the backend saw chosen.
      const file = await pickFileToRead('env')
      return file && live() ? ingestEnvFile(file) : null
    })

  return { drop, pick, error }
}
