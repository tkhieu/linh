import { describe, expect, it } from 'vitest'
import { chrome, firefox, work } from './test/fixtures'
import { deriveTargets, parseTargetKey, targetKey, visibleTargets } from './targets'

describe('browser targets', () => {
  it('derives browser, Chromium profile, and private variants in stable order', () => {
    const targets = deriveTargets([chrome, firefox, chrome])
    expect(targets.map(({ label }) => label)).toEqual([
      'Google Chrome', 'Google Chrome (Private)', 'Google Chrome — Work',
      'Google Chrome — Work (Private)', 'Firefox', 'Firefox (Private)',
    ])
    expect(targets[2].target).toEqual(work)
    expect(new Set(targets.map(({ key }) => key)).size).toBe(targets.length)
  })

  it('filters exact targets, with an empty selection meaning all', () => {
    const targets = deriveTargets([chrome, firefox])
    expect(visibleTargets(targets, [targetKey(work)]).map(({ label }) => label)).toEqual(['Google Chrome — Work'])
    expect(visibleTargets(targets, [])).toEqual(targets)
  })

  it('round-trips human-readable browser, profile, and private keys', () => {
    const cases = [
      [{ browser: 'firefox', private: false }, 'firefox'],
      [{ browser: 'firefox', private: true }, 'firefox#private'],
      [{ browser: 'google-chrome', profile: 'Profile 1', private: false }, 'google-chrome/Profile 1'],
      [{ browser: 'google-chrome', profile: 'Profile 1', private: true }, 'google-chrome/Profile 1#private'],
    ] as const
    for (const [target, key] of cases) {
      expect(targetKey(target)).toBe(key)
      expect(parseTargetKey(key)).toEqual(target)
    }
  })

  it('splits only the first profile separator and rejects malformed keys', () => {
    expect(parseTargetKey('chrome/Profile 1/Child#private')).toEqual({
      browser: 'chrome', profile: 'Profile 1/Child', private: true,
    })
    expect(parseTargetKey('chrome/')).toBeNull()
    expect(parseTargetKey('chrome#private#private')).toBeNull()
  })

  it('ignores stale keys when rendering targets', () => {
    const targets = deriveTargets([chrome, firefox])
    expect(visibleTargets(targets, ['retired-browser/Profile 9', 'firefox#private']))
      .toEqual([targets[5]])
  })
})
