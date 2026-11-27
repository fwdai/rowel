import { useEffect, useRef, useState, type RefObject } from 'react'
import { shallow } from 'zustand/shallow'
import { crossfade } from '@/utils/crossfade'
import { scrollParent } from '@/utils/scrollParent'

interface Face<T> {
  key: string
  value: T
}

export function useSwapped<T extends object>(
  value: T,
  key: string,
  ready: boolean,
  anchor: RefObject<HTMLElement | null>
): { shown: T | null; settling: boolean } {
  const [face, setFace] = useState<Face<T> | null>(ready ? { key, value } : null)
  if (ready && face?.key === key && !shallow(face.value, value)) setFace({ key, value })

  const next = useRef<Face<T> | null>(null)
  useEffect(() => {
    if (ready) next.current = { key, value }
  })

  const shownKey = face?.key
  useEffect(() => {
    if (!ready || shownKey === key) return
    const region = anchor.current && scrollParent(anchor.current)
    crossfade(region, () => setFace(next.current))
  }, [ready, key, shownKey, anchor])

  const current = ready && shownKey === key
  return { shown: current ? value : (face?.value ?? null), settling: !!face && !current }
}
