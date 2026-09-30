import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import type { TKey } from '@/i18n'
import DialogHeader from '@/components/elements/DialogHeader'
import IconButton from '@/components/elements/IconButton'
import { CARD } from '@/components/elements/tokens'
import type { GeneratorApply, SshApply } from '@/store'
import { RefreshGlyph } from '../icons'
import type { GeneratorDialog } from './useGeneratorDialog'
import Actions from './Actions'
import Amount from './Amount'
import Hero from './Hero'
import Panel from './Panel'
import Tabs from './Tabs'
import Toggles from './Toggles'

interface Props {
  generator: GeneratorDialog
  apply: GeneratorApply | null
  ssh: SshApply | null
  onClose: () => void
}

// The desktop body, laid out like a settings pane: the shared header, the mode
// switch, the secret as the hero card, its shaping controls as one card of
// rows, and the action bar pinned under a hairline.
export default function Card({ generator, apply, ssh, onClose }: Props) {
  const { t } = useTranslation()
  const { mode, setMode, keys, settings, value, bits, level, ready, update, regenerate } =
    generator

  // Says where the value goes, which is what confirming does in each case.
  const description: TKey = keys
    ? ssh
      ? 'Fills the key fields when you use it.'
      : 'Opens as a new SSH key entry.'
    : apply
      ? 'Fills the password field when you use it.'
      : 'Copied when you’re done, never saved.'

  return (
    <>
      <DialogHeader
        id="generator-title"
        title={t(keys ? 'Generate an SSH key' : 'Generate a password')}
        description={t(description)}
        onClose={onClose}
      />
      <div className="min-h-0 flex-1 overflow-y-auto px-7 pt-4 pb-6">
        {/* Opened for a key, there is nothing to switch to. */}
        {!ssh && (
          <Tabs
            mode={mode}
            ssh={!apply}
            onChange={setMode}
            className="mb-4 w-full [&>button]:flex-1"
          />
        )}
        {keys ? (
          <div className={cx(CARD, 'p-4')}>
            <Panel generator={generator} />
          </div>
        ) : (
          <>
            <Hero
              value={value}
              bits={bits}
              level={level}
              ready={ready}
              onRegenerate={regenerate}
            />
            <div className={cx(CARD, 'mt-3')}>
              <Amount settings={settings} onChange={update} row />
              <Toggles settings={settings} onChange={update} rows />
            </div>
          </>
        )}
      </div>
      <div className="flex flex-none items-center gap-1.5 border-t border-line px-7 py-3.5">
        {/* A keypair has no hero to carry its redraw, so the bar does. */}
        {keys && (
          <IconButton
            title={t('Regenerate')}
            testid="generator-regenerate"
            onClick={regenerate}
            className="h-9 w-9 border border-line2 hover:border-accent-line"
          >
            <RefreshGlyph />
          </IconButton>
        )}
        <div className="flex-1" />
        <Actions generator={generator} onClose={onClose} />
      </div>
    </>
  )
}
