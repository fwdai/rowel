import { useEffect, useState } from 'react'
import { exchangeSupported } from '@/api/imports'

// Whether another app can hand its data to this device directly. Asked once,
// when the section mounts, and false until answered: the tiles are drawn as
// every other device draws them, and only a device that can changes one.
export function useExchangeSupported(): boolean {
  const [supported, setSupported] = useState(false)
  useEffect(() => {
    let mounted = true
    exchangeSupported()
      .then(answer => {
        if (mounted) setSupported(answer)
      })
      .catch(() => {})
    return () => {
      mounted = false
    }
  }, [])
  return supported
}
