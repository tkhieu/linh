import { describe, expect, it } from 'vitest'
import type { Config } from './api'
import { parseTraceUrl, patternMatches, traceRoute } from './routing'

const caps = { source_app_rules: false }

function baseConfig(): Config {
  return { primary: { browser: 'firefox', private: false }, prompt_mode: 'never', shown_browsers: [], rules: [] }
}

describe('patternMatches', () => {
  it('plain domain matches itself and subdomains', () => {
    expect(patternMatches('company.com', 'company.com', '/')).toBe(true)
    expect(patternMatches('company.com', 'a.company.com', '/')).toBe(true)
    expect(patternMatches('company.com', 'a.b.company.com', '/')).toBe(true)
    expect(patternMatches('company.com', 'notcompany.com', '/')).toBe(false)
    expect(patternMatches('company.com', 'company.com.evil.com', '/')).toBe(false)
  })

  it('wildcard domain matches only subdomains', () => {
    expect(patternMatches('*.company.com', 'a.company.com', '/')).toBe(true)
    expect(patternMatches('*.company.com', 'a.b.company.com', '/')).toBe(true)
    expect(patternMatches('*.company.com', 'company.com', '/')).toBe(false)
  })

  it('host comparison is case-insensitive', () => {
    expect(patternMatches('Company.COM', 'company.com', '/')).toBe(true)
    expect(patternMatches('company.com', 'COMPANY.COM', '/')).toBe(true)
  })

  it('path wildcard matches prefix', () => {
    expect(patternMatches('example.com/path*', 'example.com', '/path/foo')).toBe(true)
    expect(patternMatches('example.com/path*', 'example.com', '/path')).toBe(true)
    expect(patternMatches('example.com/path*', 'example.com', '/other')).toBe(false)
  })

  it('path pattern requires domain match too', () => {
    expect(patternMatches('example.com/path*', 'notexample.com', '/path')).toBe(false)
  })

  it('meet.google.com matches', () => {
    expect(patternMatches('meet.google.com', 'meet.google.com', '/')).toBe(true)
    expect(patternMatches('meet.google.com', 'google.com', '/')).toBe(false)
  })
})

describe('parseTraceUrl', () => {
  it('returns null for empty input', () => {
    expect(parseTraceUrl('')).toBeNull()
    expect(parseTraceUrl('   ')).toBeNull()
  })

  it('assumes https for a bare host', () => {
    expect(parseTraceUrl('github.com/HuntingAlice/warmbly-setup')).toEqual({
      host: 'github.com', path: '/HuntingAlice/warmbly-setup',
    })
  })

  it('respects an explicit scheme', () => {
    expect(parseTraceUrl('http://example.com/a')).toEqual({ host: 'example.com', path: '/a' })
  })

  it('returns null for unparsable input', () => {
    expect(parseTraceUrl('::not a url::')).toBeNull()
  })
})

describe('traceRoute', () => {
  it('prompt mode always wins over rules, with no steps evaluated', () => {
    const config: Config = {
      ...baseConfig(), prompt_mode: 'always',
      rules: [{ name: 'r1', match: ['example.com'], open_in: { browser: 'chrome', private: false } }],
    }
    const trace = traceRoute(config, caps, 'example.com', '/')
    expect(trace.decision).toEqual({ kind: 'prompt-always' })
    expect(trace.steps).toEqual([])
  })

  it('first matching rule wins', () => {
    const config: Config = {
      ...baseConfig(),
      rules: [
        { name: 'general', match: ['company.com'], open_in: { browser: 'chrome', private: false } },
        { name: 'specific', match: ['a.company.com'], open_in: { browser: 'firefox', private: false } },
      ],
    }
    const trace = traceRoute(config, caps, 'a.company.com', '/')
    expect(trace.decision).toEqual({ kind: 'open', target: { browser: 'chrome', private: false } })
    expect(trace.matchedIndex).toBe(0)
    expect(trace.steps).toEqual([
      { index: 0, name: 'general', status: 'match' },
      { index: 1, name: 'specific', status: 'skip' },
    ])
  })

  it('falls back to primary when no rule matches', () => {
    const config = baseConfig()
    const trace = traceRoute(config, caps, 'unmatched.example', '/')
    expect(trace.decision).toEqual({ kind: 'open', target: config.primary })
    expect(trace.matchedIndex).toBeNull()
  })

  it('rule can resolve to an app target', () => {
    const appTarget = { app: 'teams' as const, fallback: { browser: 'google-chrome', private: false } }
    const config: Config = {
      ...baseConfig(),
      rules: [{ name: 'teams', match: ['teams.microsoft.com/l/meetup-join/*'], open_in: appTarget }],
    }
    const trace = traceRoute(config, caps, 'teams.microsoft.com', '/l/meetup-join/19%3aabc')
    expect(trace.decision).toEqual({ kind: 'open-app', app: appTarget })
  })

  it('marks rules after the winner as skipped without evaluating them', () => {
    const config: Config = {
      ...baseConfig(),
      rules: [
        { name: 'first', match: ['example.com'], open_in: { browser: 'chrome', private: false } },
        { name: 'unreachable', match: ['example.com'], source_apps: ['Slack'], open_in: { browser: 'firefox', private: false } },
      ],
    }
    const trace = traceRoute(config, caps, 'example.com', '/')
    expect(trace.steps).toEqual([
      { index: 0, name: 'first', status: 'match' },
      { index: 1, name: 'unreachable', status: 'skip' },
    ])
  })

  it('rule can resolve to prompt', () => {
    const config: Config = { ...baseConfig(), rules: [{ name: 'ask', match: ['example.com'], open_in: 'prompt' }] }
    const trace = traceRoute(config, caps, 'example.com', '/')
    expect(trace.decision).toEqual({ kind: 'prompt' })
  })

  it('source app rule skipped without capability', () => {
    const config: Config = {
      ...baseConfig(),
      rules: [{ name: 'slack-rule', match: ['example.com'], source_apps: ['Slack'], open_in: { browser: 'chrome', private: false } }],
    }
    const trace = traceRoute(config, { source_app_rules: false }, 'example.com', '/')
    expect(trace.decision).toEqual({ kind: 'open', target: config.primary })
    expect(trace.steps).toEqual([{ index: 0, name: 'slack-rule', status: 'skip-no-source-app-caps' }])
  })

  it('source app rule evaluated when capability present', () => {
    const config: Config = {
      ...baseConfig(),
      rules: [{ name: 'slack-rule', match: ['example.com'], source_apps: ['Slack'], open_in: { browser: 'chrome', private: false } }],
    }
    const trace = traceRoute(config, { source_app_rules: true }, 'example.com', '/')
    expect(trace.decision).toEqual({ kind: 'open', target: { browser: 'chrome', private: false } })
  })
})
