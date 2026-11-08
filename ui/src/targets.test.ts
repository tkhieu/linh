import { describe, expect, it } from 'vitest'
import { chrome, firefox, work } from './test/fixtures'
import { deriveTargets, targetKey, visibleTargets } from './targets'

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
})
