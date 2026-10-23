import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { closePasswordPrompt } from '@/store'
import Frame from '@/components/elements/Frame'
import AdoptForm from './AdoptForm'
import Unmoved from './Unmoved'

const TITLE_ID = 'password-changed-title'

/**
 * Ask for the new master password once, then either close (everything moved
 * with it) or say which lockboxes beside this one stayed on a password of
 * their own and will ask for it when they are next opened.
 */
export default function PasswordChangedDialog() {
  const { t } = useTranslation()
  // The ids `syncAdoptPassword` could not move, once it has landed.
  const [unmoved, setUnmoved] = useState<string[] | null>(null)

  const adopted = (ids: string[]) => {
    if (ids.length === 0) closePasswordPrompt()
    else setUnmoved(ids)
  }

  return (
    <Frame
      onClose={closePasswordPrompt}
      labelledBy={TITLE_ID}
      testid="password-changed-modal"
      fit="content"
      className="w-dialog"
    >
      <div className="p-7">
        <h2 id={TITLE_ID} className="text-lg font-semibold tracking-display">
          {t('Master password changed on another device')}
        </h2>
        {unmoved ? (
          <Unmoved ids={unmoved} />
        ) : (
          <>
            <p className="mt-1.5 text-base text-text2">
              {t(
                'Enter the new master password to keep this lockbox in sync. This device will switch to it.'
              )}
            </p>
            <AdoptForm onAdopted={adopted} />
          </>
        )}
      </div>
    </Frame>
  )
}
