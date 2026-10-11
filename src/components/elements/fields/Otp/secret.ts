const BASE32 = /^[A-Z2-7]{8,}$/

/** The `secret` parameter of an otpauth:// URI, whatever case it was spelled in. */
const secretParam = (query: string): string => {
  for (const [key, value] of new URLSearchParams(query)) {
    if (key.toLowerCase() === 'secret') return value
  }
  return ''
}

/**
 * What the backend will generate for (`otp.rs`: DIGITS, PERIOD, the three RFC
 * 6238 hashes). Mirrored here so a link the backend would refuse is red in the
 * field instead of saved and shown as an empty dial with no explanation.
 */
const SUPPORTED: Record<string, (param: string) => boolean> = {
  digits: p => /^\d+$/.test(p) && Number(p) >= 6 && Number(p) <= 10,
  period: p => /^\d+$/.test(p) && Number(p) >= 1 && Number(p) <= 300,
  algorithm: p => ['sha1', 'sha256', 'sha512'].includes(p.toLowerCase())
}

/**
 * The parameters that decide which code comes out. Each is read once: with two
 * `secret`s, a first-wins reader and a last-wins reader pass the same link and
 * generate different codes from it — this check used to read the first and the
 * backend the last, so a good seed followed by junk was green here and empty
 * on the dial. A link that repeats one is refused, as `otp.rs` refuses it.
 */
const DECISIVE = ['secret', ...Object.keys(SUPPORTED)]

/**
 * The TOTP secret carried by whatever was pasted: a bare base32 string, or the
 * `otpauth://totp/...?secret=...` URI behind every enrolment QR code. Returns
 * '' when there is no usable secret in there — the field's validity test. A
 * link is usable only if it is time-based (an `hotp` seed run through the
 * clock gives plausible, wrong codes) and every parameter it spells out is one
 * the backend accepts.
 */
export const otpSecret = (value: string): string => {
  const raw = value.trim().replace(/\s+/g, '')
  if (!raw) return ''
  if (/^otpauth:\/\//i.test(raw)) {
    const type = raw.slice('otpauth://'.length).split(/[/?]/, 1)[0] ?? ''
    if (type.toLowerCase() !== 'totp') return ''
    const query = raw.includes('?') ? raw.slice(raw.indexOf('?') + 1) : ''
    const seen = new Set<string>()
    for (const [key, param] of new URLSearchParams(query)) {
      const name = key.toLowerCase()
      if (DECISIVE.includes(name)) {
        if (seen.has(name)) return ''
        seen.add(name)
      }
      const accepts = SUPPORTED[name]
      if (accepts && !accepts(param.trim())) return ''
    }
    return otpSecret(secretParam(query))
  }
  // The backend decodes with base32 RFC4648 padding *off* (totp-rs
  // `Secret::Encoded`), which rejects '=' outright — so padding is dropped
  // here rather than stored and refused later.
  const upper = raw.toUpperCase().replace(/=+$/, '')
  return BASE32.test(upper) ? upper : ''
}

/** What each otpauth:// parameter means when it is not spelled out. */
const DEFAULTS: Record<string, string> = { digits: '6', period: '30', algorithm: 'sha1' }

/**
 * The canonical value to store for whatever was pasted: the bare secret when
 * the code is generated with the defaults, and the whole otpauth:// URI when it
 * carries a digit count, period or algorithm that differs.
 *
 * Those parameters are as much a part of the enrolment as the seed — an 8-digit
 * or 60-second QR code reduced to its secret alone produces codes the site
 * refuses, silently, which is exactly how it used to lock people out. The bare
 * form stays for everything else so the common case keeps its short string.
 */
export const otpStored = (value: string): string => {
  const raw = value.trim()
  const secret = otpSecret(raw)
  if (!secret || !/^otpauth:\/\//i.test(raw) || !raw.includes('?')) return secret
  for (const [key, param] of new URLSearchParams(raw.slice(raw.indexOf('?') + 1))) {
    const expected = DEFAULTS[key.toLowerCase()]
    if (expected !== undefined && param.trim().toLowerCase() !== expected) return raw
  }
  return secret
}

// A code split down the middle for reading: three and three for the usual
// six, four and four for an 8-digit seed, rather than 3 + everything else. The
// row and the dial both read a code this way.
export const groupDigits = (code: string): string => {
  const half = Math.ceil(code.length / 2)
  return `${code.slice(0, half)} ${code.slice(half)}`
}
