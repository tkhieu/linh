// Mirrors the Tauri command contract frozen in `src-tauri/src/commands.rs`.
// Keep these types and wrappers in lockstep with that file: phases 2 (Rust
// backend) and 3 (this UI) both build against this contract.
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

/** Either `"prompt"` or a concrete `Target`. */
export type OpenIn = 'prompt' | Target

export type PromptMode = 'never' | 'always'

export interface Rule {
  name: string
  match: string[]
  source_apps?: string[]
  open_in: OpenIn
}

export interface Config {
  primary: Target
  prompt_mode: PromptMode
  shown_browsers: string[]
  rules: Rule[]
}

export interface PlatformCaps {
  source_app_rules: boolean
}

export function listBrowsers(): Promise<Browser[]> {
  return invoke('list_browsers')
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
