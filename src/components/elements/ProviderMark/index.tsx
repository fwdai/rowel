import type { Provider } from '@/api/provider'
import { cx } from '@/utils/cx'
import { ServerGlyph } from '@/components/Main/icons'
import GoogleDriveMark from './GoogleDriveMark'
import DropboxMark from './DropboxMark'
import ICloudMark from './ICloudMark'

interface Props {
  provider: Provider
  size?: number
  className?: string
}

/**
 * The provider's own mark, in its brand colours wherever it is drawn. A Rowel
 * Server is a company's own machine with no brand of its own, so it wears the
 * set's server glyph in whatever ink the place it sits in gives it.
 */
export default function ProviderMark({ provider, size, className }: Props) {
  switch (provider) {
    case 'googleDrive':
      return <GoogleDriveMark size={size} className={className} />
    case 'dropbox':
      return <DropboxMark size={size} className={cx('text-[#0061fe]', className)} />
    case 'iCloud':
      return <ICloudMark size={size} className={cx('text-[#3693f3]', className)} />
    case 'rowelServer':
      return <ServerGlyph size={size} className={className} />
  }
}
