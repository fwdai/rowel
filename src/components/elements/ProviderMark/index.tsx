import type { Provider } from '@/api/provider'
import { cx } from '@/utils/cx'
import GoogleDriveMark from './GoogleDriveMark'
import DropboxMark from './DropboxMark'
import ICloudMark from './ICloudMark'

interface Props {
  provider: Provider
  size?: number
  className?: string
}

/** The provider's own mark, in its brand colours wherever it is drawn. */
export default function ProviderMark({ provider, size, className }: Props) {
  switch (provider) {
    case 'googleDrive':
      return <GoogleDriveMark size={size} className={className} />
    case 'dropbox':
      return <DropboxMark size={size} className={cx('text-[#0061fe]', className)} />
    case 'iCloud':
      return <ICloudMark size={size} className={cx('text-[#3693f3]', className)} />
  }
}
