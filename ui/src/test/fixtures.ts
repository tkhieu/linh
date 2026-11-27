import type { Browser, Config, Target } from '../api'

export const firefox: Browser = {
  id: 'firefox', display_name: 'Firefox', family: 'firefox',
  install: { native: { exec: 'firefox %u' } }, profiles: [], supports_private: true,
}

export const chrome: Browser = {
  id: 'chrome', display_name: 'Google Chrome', family: 'chromium',
  install: { native: { exec: 'google-chrome %u' } },
  profiles: [{ dir: 'Profile 1', display_name: 'Work' }], supports_private: true,
}

export const work: Target = { browser: 'chrome', profile: 'Profile 1', private: false }

export const config: Config = {
  primary: { browser: 'firefox', private: false },
  prompt_mode: 'never', shown_browsers: [], rules: [], check_for_updates: true,
}
