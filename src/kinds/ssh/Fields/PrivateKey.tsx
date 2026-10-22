import { useState, type CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'
import { openSshGenerator } from '@/store'
import IconButton from '@/components/elements/IconButton'
import { FieldRow, grow, useField, useFields } from '@/components/elements/fields'
import { requiredError } from '@/components/elements/fields/formats'
import { verbatimInput } from '@/components/elements/inputProps'
import { EyeGlyph, EyeOffGlyph, RefreshGlyph } from '@/components/Main/icons'

// A textarea cannot fake dots, so the editor hides its own text.
const MASK = { WebkitTextSecurity: 'disc' } as CSSProperties

// The OpenSSH private block's editor: multiline, and the one row the generator
// writes back through — a keypair is three draft keys, so the Generate button
// takes the field-set writer rather than this row's own setter. Editing only:
// the read view seals the block instead (see PrivateBlock).
export default function PrivateKey() {
  const { t } = useTranslation()
  const { set: write } = useFields()
  const { value, set, attempted } = useField('privateKey')
  const [show, setShow] = useState(true)

  const generate = () =>
    openSshGenerator(pair => {
      write?.('privateKey', pair.privateKey)
      write?.('publicKey', pair.publicKey)
      write?.('fingerprint', pair.fingerprint)
    })

  return (
    <FieldRow
      label="Private key"
      error={requiredError(value, true, attempted)}
      actions={
        <>
          <IconButton title={t('Generate')} testid="generate-ssh-key-link" onClick={generate}>
            <RefreshGlyph />
          </IconButton>
          <IconButton
            title={show ? t('Hide') : t('Reveal')}
            active={show}
            testid="reveal-privateKey"
            onClick={() => setShow(!show)}
          >
            {show ? <EyeOffGlyph /> : <EyeGlyph />}
          </IconButton>
        </>
      }
    >
      {id => (
        <textarea
          id={id}
          name="privateKey"
          value={value}
          rows={1}
          placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
          {...verbatimInput}
          ref={grow}
          style={show ? undefined : MASK}
          onChange={event => {
            grow(event.currentTarget)
            set(event.target.value)
          }}
          // Three lines deep before anything is pasted: a block's editor
          // reads as a block, never as a one-line field for a PEM file.
          className="block min-h-[72px] w-full resize-none overflow-hidden border-b border-line2 bg-transparent font-mono text-base leading-relaxed text-text outline-none transition-colors placeholder:text-text2 focus:border-accent-line"
        />
      )}
    </FieldRow>
  )
}
