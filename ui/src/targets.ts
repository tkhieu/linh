import type { Browser, Target } from './api'

export interface BrowserTarget {
  key: string
  label: string
  target: Target
}

/** An exact, collision-free identity for entries stored in shown_browsers. */
export function targetKey(target: Target): string {
  return JSON.stringify([target.browser, target.profile ?? null, target.private])
}

export function deriveTargets(browsers: Browser[]): BrowserTarget[] {
  const results: BrowserTarget[] = []
  const seen = new Set<string>()
  const add = (target: Target, label: string) => {
    const key = targetKey(target)
    if (!seen.has(key)) {
      seen.add(key)
      results.push({ key, label, target })
    }
  }

  for (const browser of browsers) {
    const variants: { target: Target; label: string }[] = [{ target: { browser: browser.id, private: false }, label: browser.display_name }]
    if (browser.family === 'chromium') {
      for (const profile of browser.profiles) {
        variants.push({
          target: { browser: browser.id, profile: profile.dir, private: false },
          label: `${browser.display_name} — ${profile.display_name}`,
        })
      }
    }
    for (const variant of variants) {
      add(variant.target, variant.label)
      if (browser.supports_private) {
        add({ ...variant.target, private: true }, `${variant.label} (Private)`)
      }
    }
  }
  return results
}

export function visibleTargets(targets: BrowserTarget[], shown: string[]): BrowserTarget[] {
  return shown.length === 0 ? targets : targets.filter(({ key }) => shown.includes(key))
}
