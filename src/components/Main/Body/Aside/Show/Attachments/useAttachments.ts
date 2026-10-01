import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Attachment } from '@/api/types'
import {
  addAttachment,
  deleteAttachment,
  listAttachments,
  saveAttachment
} from '@/api/attachments'
import { pickFileToRead } from '@/api/tools'
import { describeError, errorKind } from '@/api/errors'

/**
 * One entry's attachments and what can be done to them. The list is the
 * entry's own and nobody else draws it, so it lives here rather than in the
 * vault store: read when the entry opens, kept in step by this hook's own
 * writes, gone when the entry closes.
 */
export function useAttachments(entryId: string) {
  const { t } = useTranslation()
  const [items, setItems] = useState<Attachment[]>([])
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let alive = true
    setItems([])
    setError(null)
    listAttachments(entryId)
      .then(listed => alive && setItems(listed))
      .catch(() => {})
    return () => {
      alive = false
    }
  }, [entryId])

  const fail = (e: unknown) =>
    setError(
      errorKind(e) === 'fileTooLarge'
        ? t('Attachments can be up to {{size}} each', { size: '10 MB' })
        : describeError(e) || t('Something went wrong')
    )

  /** Attach `path`, or the file the user picks when there is none. */
  const add = async (path?: string) => {
    setError(null)
    const picked = path ?? (await pickFileToRead('attachment').catch(() => null))
    if (!picked) return
    setBusy(true)
    try {
      const added = await addAttachment(entryId, picked)
      setItems(current => [...current, added])
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }

  const save = (id: string) => {
    setError(null)
    saveAttachment(id).catch(fail)
  }

  const remove = (id: string) => {
    setError(null)
    deleteAttachment(id)
      .then(() => setItems(current => current.filter(item => item.id !== id)))
      .catch(fail)
  }

  return { items, error, busy, add, save, remove }
}
