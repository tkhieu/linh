import type { Browser, Target } from './api'

export interface BrowserTarget {
  key: string
  label: string
  target: Target
}

/** Human-readable target identity used only by shown_browsers. */
export function targetKey(target: Target): string {
  return `${target.browser}${target.profile === undefined ? '' : `/${target.profile}`}${target.private ? '#private' : ''}`
}

export function parseTargetKey(key: string): Target | null {
  const privateSuffix = '#private'
  const suffixAt = key.indexOf(privateSuffix)
  if (suffixAt !== -1 && suffixAt !== key.length - privateSuffix.length) return null
  const isPrivate = suffixAt !== -1
  const base = isPrivate ? key.slice(0, suffixAt) : key
  const profileAt = base.indexOf('/')
  if (!base || profileAt === 0 || profileAt === base.length - 1) return null
  return profileAt === -1
    ? { browser: base, private: isPrivate }
    : { browser: base.slice(0, profileAt), profile: base.slice(profileAt + 1), private: isPrivate }
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
  if (shown.length === 0) return targets
  const selected = new Set(shown.flatMap((key) => {
    const target = parseTargetKey(key)
    return target ? [targetKey(target)] : []
  }))
  const filtered = targets.filter(({ key }) => selected.has(key))
  // shown_browsers names only browsers that are no longer installed: show
  // every detected target rather than an empty picker.
  return filtered.length === 0 && targets.length > 0 ? targets : filtered
}
