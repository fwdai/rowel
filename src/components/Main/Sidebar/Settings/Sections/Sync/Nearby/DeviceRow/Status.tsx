import { useTranslation } from 'react-i18next'
import type { NearbyDevice } from '@/api/nearby'
import { useDates } from '@/hooks/useDates'
import { statusLine } from './statusLine'

export default function Status({ device }: { device: NearbyDevice }) {
  const { t } = useTranslation()
  const { relativeLong } = useDates()
  const { text, bad } = statusLine(t, device, relativeLong)

  return (
    <span
      data-testid={`settings-nearby-status-${device.id}`}
      className={bad ? 'text-bad' : undefined}
    >
      {text}
    </span>
  )
}
