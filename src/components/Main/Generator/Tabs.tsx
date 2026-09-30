import { useTranslation } from 'react-i18next'
import Segmented from '@/components/elements/Segmented'
import type { TKey } from '@/i18n'
import type { GeneratorMode } from '@/services/generator'

/**
 * What the dialog is showing. The password modes are the generator's own
 * settings (see services/generator); `ssh` is a face of the dialog rather than
 * a fourth password shape, so it stays out of the persisted defaults.
 */
export type DialogMode = GeneratorMode | 'ssh'

const TABS: { value: DialogMode; label: TKey }[] = [
  { value: 'random', label: 'Random' },
  { value: 'memorable', label: 'Memorable' },
  { value: 'ssh', label: 'SSH key' }
]

interface Props {
  mode: DialogMode
  onChange: (mode: DialogMode) => void
  /** Offered only when nothing is waiting for a password: a password field has
   *  no use for a keypair. */
  ssh?: boolean
  /** The switch's box. It sits beside a title in a phone's sheet and spans the
   *  width of the desktop card and the phone's tab root, so the size travels as
   *  classes. */
  className?: string
}

// The generator's mode switch — the shared Segmented control.
export default function Tabs({ mode, onChange, ssh, className = 'flex-none' }: Props) {
  const { t } = useTranslation()
  const tabs = ssh ? TABS : TABS.filter(tab => tab.value !== 'ssh')
  return (
    <Segmented
      options={tabs.map(tab => ({ value: tab.value, label: t(tab.label) }))}
      value={mode}
      onChange={onChange}
      testidPrefix="generator-mode"
      className={className}
    />
  )
}
