import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Attachment, AttachmentUsage } from '@/api/types'
import {
  MAX_ATTACHMENT_BYTES,
  attachmentUsage,
  listAttachments,
  saveAttachment
} from '@/api/attachments'
import { pickFileToRead } from '@/api/tools'
import { shrinkPhoto } from '@/lib/photo'
import { describeError, errorKind } from '@/api/errors'
import { attachBytes, attachFile, removeAttachment, useVault } from '@/store'

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

  const tooLarge = () =>
    setError(t('Attachments can be up to {{size}} each', { size: '10 MB' }))

  const fail = (e: unknown) =>
    errorKind(e) === 'fileTooLarge'
      ? tooLarge()
      : setError(describeError(e) || t('Something went wrong'))

  const attach = async (work: () => Promise<void>) => {
    setError(null)
    setBusy(true)
    try {
      await work()
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }

  /** Attach `path`, a file the user already chose (a drop on the window). */
  const add = (path: string) => attach(() => attachFile(entryId, path))

  /**
   * Attach a file the webview holds — a photo the phone's camera just took
   * through the capture input, cut down first (`shrinkPhoto`). Refused by size
   * before it is read: the backend would refuse it too, but not before the
   * whole thing had been loaded and sent over.
   */
  const addFile = (file: File) =>
    attach(async () => {
      const photo = await shrinkPhoto(file)
      if (photo.size > MAX_ATTACHMENT_BYTES) return tooLarge()
      const bytes = new Uint8Array(await photo.arrayBuffer())
      await attachBytes(entryId, photo.name, bytes)
    })

  /**
   * Let the user pick a file and attach it: any file from the file browser, or
   * — `from: 'photo'` — an image from the photo library, where there is one
   * (see `pickFileToRead`). The image filter's name is the only part of that
   * dialog that is ours to translate. Resolves to whether a file was picked,
   * so the phone's sheet can stay up over a cancelled picker.
   */
  const pick = async (from: AttachSource = 'attachment'): Promise<boolean> => {
    setError(null)
    const label = from === 'photo' ? t('Images') : undefined
    const picked = await pickFileToRead(from, label).catch(() => null)
    if (picked) await add(picked)
    return picked !== null
  }

  const save = (id: string) => {
    setError(null)
    saveAttachment(id).catch(fail)
  }

  const remove = (id: string) => {
    setError(null)
    removeAttachment(id).catch(fail)
  }

  return { items, usage, error, busy, add, addFile, pick, save, remove }
}
