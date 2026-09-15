/**
 * Where this build lives on the internet.
 *
 * Kept in one place because three things read it: the settings footer shows the
 * version, the "开源主页" button opens the repo, and the README quotes it.
 * Bumping a release means editing this line plus `package.json`,
 * `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` - all four are
 * checked by `scripts/check-version.mjs` so they cannot drift apart silently.
 */

/** Semantic version of the app. Must match `tauri.conf.json`. */
export const APP_VERSION = "0.4.5";

/** Public repository - the target of the "开源主页" button. */
export const REPO_URL = "https://github.com/WW-Ares/WSight";

/** Releases page, offered as a secondary link. */
export const RELEASES_URL = `${REPO_URL}/releases`;

/** Issues page, offered as a secondary link. */
export const ISSUES_URL = `${REPO_URL}/issues`;

/** Shown under the settings title: `WSight v0.4.3`. */
export const APP_VERSION_LABEL = `v${APP_VERSION}`;
