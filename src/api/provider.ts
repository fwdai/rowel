/**
 * The cloud a workspace syncs through. Serialized as Rust's `Provider` enum is
 * (camelCase), and named on every connect command, on `sync:status`, on each
 * workspace in `app_status` and on every pack a probe lists.
 */
export type Provider = 'googleDrive' | 'dropbox'

/** In the order the pickers offer them. */
export const PROVIDERS: readonly Provider[] = ['googleDrive', 'dropbox']

/** Product names, so never translated: copy interpolates them as `{{provider}}`. */
const NAMES: Record<Provider, string> = {
  googleDrive: 'Google Drive',
  dropbox: 'Dropbox'
}

export const providerName = (provider: Provider): string => NAMES[provider]

/**
 * The short form test ids are built from. Google's stays `drive`, which is
 * what every id from before there was a second provider already says.
 */
export const providerSlug = (provider: Provider): string =>
  provider === 'googleDrive' ? 'drive' : provider

/**
 * Shares are Drive files read through a public link, and the link format has
 * no room yet to say which provider to fetch from, so only Drive shares.
 */
export const providerShares = (provider: Provider | null): boolean => provider === 'googleDrive'
