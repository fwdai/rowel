/**
 * Where a workspace syncs through. Serialized as Rust's `Provider` enum is
 * (camelCase), and named on every connect command, on `sync:status`, on each
 * workspace in `app_status` and on every pack a probe lists. The clouds are
 * accounts the user owns; `rowelServer` is a company's server, which an org
 * lockbox syncs through and which is joined from Settings › Lockboxes rather
 * than picked from a connect button.
 */
export type Provider = 'googleDrive' | 'dropbox' | 'iCloud' | 'rowelServer'

/**
 * Every cloud the pickers may offer, in their order. Which of them *this* build
 * can connect is the backend's answer (`AppStatus.providers`, read through
 * `useProviders`): iCloud exists only on an Apple build signed for it, and the
 * frontend does not second-guess that with a platform check. A Rowel Server is
 * never in that answer: there is no account to pick, only a URL to sign in at.
 */
export const PROVIDERS: readonly Provider[] = ['googleDrive', 'dropbox', 'iCloud']

/** Product names, so never translated: copy interpolates them as `{{provider}}`. */
const NAMES: Record<Provider, string> = {
  googleDrive: 'Google Drive',
  dropbox: 'Dropbox',
  iCloud: 'iCloud',
  rowelServer: 'Rowel Server'
}

export const providerName = (provider: Provider): string => NAMES[provider]

/**
 * The short form test ids are built from. Google's stays `drive`, which is
 * what every id from before there was a second provider already says; the
 * rest are lower-cased, since an id reads as one word.
 */
export const providerSlug = (provider: Provider): string =>
  provider === 'googleDrive' ? 'drive' : provider.toLowerCase()

/**
 * Whether a connect sends the user out to a browser for consent. iCloud is the
 * signed-in Apple account the OS already holds, so its connect is a check that
 * answers on its own — the waiting copy should not tell the user to look for a
 * browser that never opens.
 */
export const providerOpensBrowser = (provider: Provider): boolean => provider !== 'iCloud'

/**
 * Shares are Drive files read through a public link, and the link format has
 * no room yet to say which provider to fetch from, so only Drive shares.
 */
export const providerShares = (provider: Provider | null): boolean => provider === 'googleDrive'
