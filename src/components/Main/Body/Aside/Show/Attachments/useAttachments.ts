import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Attachment, AttachmentUsage } from '@/api/types'
import { attachmentUsage, listAttachments, saveAttachment } from '@/api/attachments'
import { pickFileToRead } from '@/api/tools'
import { describeError, errorKind } from '@/api/errors'
import { attachFile, removeAttachment, useVault } from '@/store'

/** Where a picked attachment comes from: the file browser, or the photo library. */
export type AttachSource = 'attachment' | 'photo'

/**
 * One entry's attachments and what can be done to them. The list is the
 * entry's own and nobody else draws it, so it lives here rather than in the
 * vault store — but it is read again on the store's `revision`, which moves
 * with this hook's own writes (made through the store, so they are published)
 * and with a sync merge, which can add or remove files while the entry stays
 * open. The vault's budget is read with it.
 */
export function useAttachments(entryId: string) {
  const { t } = useTranslation()
  const revision = useVault(state => state.revision)
  const [items, setItems] = useState<Attachment[]>([])
  const [usage, setUsage] = useState<AttachmentUsage | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    setItems([])
    setError(null)
  }, [entryId])

  useEffect(() => {
    let alive = true
    listAttachments(entryId)
      .then(listed => alive && setItems(listed))
      .catch(() => {})
    attachmentUsage()
      .then(used => alive && setUsage(used))
      .catch(() => {})
    return () => {
      alive = false
    }
  }, [entryId, revision])

  const fail = (e: unknown) =>
    setError(
      errorKind(e) === 'fileTooLarge'
        ? t('Attachments can be up to {{size}} each', { size: '10 MB' })
        : describeError(e) || t('Something went wrong')
    )

  /** Attach `path`, a file the user already chose (a drop on the window). */
  const add = async (path: string) => {
    setError(null)
    setBusy(true)
    try {
      await attachFile(entryId, path)
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }

  /**
   * Let the user pick a file and attach it: any file from the file browser, or
   * — `from: 'photo'` — an image from the photo library, where there is one
   * (see `pickFileToRead`). The image filter's name is the only part of that
   * dialog that is ours to translate.
   */
  const pick = async (from: AttachSource = 'attachment') => {
    setError(null)
    const label = from === 'photo' ? t('Images') : undefined
    const picked = await pickFileToRead(from, label).catch(() => null)
    if (picked) await add(picked)
  }

  const save = (id: string) => {
    setError(null)
    saveAttachment(id).catch(fail)
  }

  const remove = (id: string) => {
    setError(null)
    removeAttachment(id).catch(fail)
  }

  return { items, usage, error, busy, add, pick, save, remove }
}
