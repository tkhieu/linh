// Mirrors the Tauri command contract frozen in the presets and UI polish plan.
import { invoke } from '@tauri-apps/api/core'

export type BrowserFamily = 'chromium' | 'firefox' | 'other'

export type InstallKind =
  | { native: { exec: string } }
  | { flatpak: { app_id: string } }

export interface BrowserProfile {
  dir: string
  display_name: string
}

export interface Browser {
  id: string
  display_name: string
  family: BrowserFamily
  install: InstallKind
  profiles: BrowserProfile[]
  supports_private: boolean
  icon?: string
  desktop_file?: string
}

export interface Target {
  browser: string
  profile?: string
  private: boolean
}

export interface AppTarget {
  app: 'teams' | 'zoom'
  fallback?: Target
}

export type OpenIn = 'prompt' | Target | AppTarget

export interface Preset {
  id: string
  name: string
  patterns: string[]
  app?: string | null
  app_installed: boolean
}

export type PromptMode = 'never' | 'always'

export interface Rule {
  name: string
  preset?: 'meet' | 'teams' | 'slack' | 'zoom'
  match: string[]
  source_apps?: string[]
  open_in: OpenIn
}

export interface Config {
  primary: Target
  prompt_mode: PromptMode
  shown_browsers: string[]
  rules: Rule[]
  check_for_updates: boolean
}

export interface PlatformCaps {
  source_app_rules: boolean
}

export type UpdateKind = 'rpm' | 'deb' | 'nsis' | 'msix' | 'mac_app' | 'dev'

export type UpdateCapability = 'self_install' | 'download_only' | 'none'

/** Set when this build's `LINH_DIST` channel is not `github`: the store or distro, not Linh, delivers updates. */
export type ManagedBy = 'microsoft_store' | 'mac_app_store' | 'flathub' | 'copr' | 'ppa'

export interface AvailableUpdate {
  version: string
  notes?: string | null
  /** RFC 3339, when the endpoint provided one. */
  date?: string | null
}

export interface UpdateStatus {
  current_version: string
  kind: UpdateKind
  capability: UpdateCapability
  managed_by: ManagedBy | null
  /** Milliseconds since the Unix epoch, when a check has run. */
  last_checked?: number | null
  available?: AvailableUpdate | null
  last_error?: string | null
}

/** Payload of the `updates://download-progress` event fired during `installUpdate`. */
export interface UpdateDownloadProgress {
  downloaded: number
  total?: number | null
}

/** Event fired whenever `checkForUpdatesNow`/`installUpdate` finish; re-fetch `getUpdateStatus` on it rather than polling. */
export const UPDATE_STATUS_CHANGED_EVENT = 'updates://status-changed'
export const UPDATE_DOWNLOAD_PROGRESS_EVENT = 'updates://download-progress'

export function listBrowsers(): Promise<Browser[]> {
  return invoke('list_browsers')
}

export function listPresets(): Promise<Preset[]> {
  return invoke('list_presets')
}

export function getIcon(icon: string): Promise<string | null> {
  return invoke('get_icon', { icon })
}

export function getConfig(): Promise<Config> {
  return invoke('get_config')
}

export function saveConfig(config: Config): Promise<void> {
  return invoke('save_config', { config })
}

export function getPlatformCaps(): Promise<PlatformCaps> {
  return invoke('get_platform_caps')
}

export function isDefaultBrowser(): Promise<boolean> {
  return invoke('is_default_browser')
}

export function setDefaultBrowser(): Promise<void> {
  return invoke('set_default_browser')
}

export function getPromptUrl(): Promise<string | null> {
  return invoke('get_prompt_url')
}

export function promptChoose(target: Target): Promise<void> {
  return invoke('prompt_choose', { target })
}

export function promptCancel(): Promise<void> {
  return invoke('prompt_cancel')
}

export function getUpdateStatus(): Promise<UpdateStatus> {
  return invoke('get_update_status')
}

export function checkForUpdatesNow(): Promise<AvailableUpdate | null> {
  return invoke('check_for_updates_now')
}

/** `version` must be the version last shown by `getUpdateStatus`/`checkForUpdatesNow`; a mismatch (e.g. a newer release appeared) is refused so the caller can re-check and re-prompt instead. */
export function installUpdate(version: string): Promise<void> {
  return invoke('install_update', { version })
}

export function openReleasePage(): Promise<void> {
  return invoke('open_release_page')
}
