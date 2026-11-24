import { describe, expect, it } from 'vitest'
import type { Config } from './api'
import { parseTraceUrl, patternMatches, resolveAppTarget, traceRoute, zoomAppUrl } from './routing'

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

  it('never prefixes https:// onto a bare host, matching url_intake (not routable)', () => {
    expect(parseTraceUrl('github.com/HuntingAlice/warmbly-setup')).toBeNull()
  })

  it('respects an explicit http/https scheme', () => {
    expect(parseTraceUrl('http://example.com/a')).toEqual({
      host: 'example.com', path: '/a', href: 'http://example.com/a',
    })
    expect(parseTraceUrl('https://example.com/a')).toEqual({
      host: 'example.com', path: '/a', href: 'https://example.com/a',
    })
  })

  it('excludes the port from host, matching Url::host_str()', () => {
    expect(parseTraceUrl('https://example.com:8443/a')).toEqual({
      host: 'example.com', path: '/a', href: 'https://example.com:8443/a',
    })
  })

  it('lowercases the host', () => {
    expect(parseTraceUrl('https://EXAMPLE.com/a')?.host).toBe('example.com')
  })

  it('rejects non-http(s) schemes, matching url_intake::filter_routable_url', () => {
    expect(parseTraceUrl('mailto:foo@bar.com')).toBeNull()
    expect(parseTraceUrl('ftp://example.com/file')).toBeNull()
    expect(parseTraceUrl('file:///etc/passwd')).toBeNull()
  })

  it('returns null for unparsable input', () => {
    expect(parseTraceUrl('::not a url::')).toBeNull()
    expect(parseTraceUrl('not a url')).toBeNull()
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

describe('zoomAppUrl', () => {
  it('rewrites subdomain host with pwd', () => {
    expect(zoomAppUrl('https://us02web.zoom.us/j/123456789?pwd=abc'))
      .toBe('zoommtg://us02web.zoom.us/join?action=join&confno=123456789&pwd=abc')
  })

  it('rewrites bare domain without pwd', () => {
    expect(zoomAppUrl('https://zoom.us/j/123456789'))
      .toBe('zoommtg://zoom.us/join?action=join&confno=123456789')
  })

  it('preserves pwd value verbatim', () => {
    expect(zoomAppUrl('https://zoom.us/j/42?pwd=a%2Fb'))
      .toBe('zoommtg://zoom.us/join?action=join&confno=42&pwd=a%2Fb')
  })

  it('ignores other query params and keeps pwd', () => {
    expect(zoomAppUrl('https://zoom.us/j/42?uname=Jane&pwd=xyz'))
      .toBe('zoommtg://zoom.us/join?action=join&confno=42&pwd=xyz')
  })

  it('rejects non-zoom host', () => {
    expect(zoomAppUrl('https://notzoom.us/j/123')).toBeNull()
  })

  it('rejects a host merely containing zoom.us as a suffix of a label', () => {
    expect(zoomAppUrl('https://evilzoom.us/j/123')).toBeNull()
  })

  it('rejects non-digit meeting id', () => {
    expect(zoomAppUrl('https://zoom.us/j/abc123')).toBeNull()
  })

  it('rejects empty meeting id', () => {
    expect(zoomAppUrl('https://zoom.us/j/')).toBeNull()
  })

  it('rejects non-join path', () => {
    expect(zoomAppUrl('https://zoom.us/pricing')).toBeNull()
  })

  it('rejects extra path segments after the id', () => {
    expect(zoomAppUrl('https://zoom.us/j/123/extra')).toBeNull()
  })

  it('rejects unparseable url', () => {
    expect(zoomAppUrl('not a url')).toBeNull()
  })

  it('rejects non-http scheme', () => {
    expect(zoomAppUrl('zoommtg://zoom.us/j/123')).toBeNull()
  })
})

describe('resolveAppTarget', () => {
  const primary = { browser: 'firefox', private: false }

  it('teams launches when installed, regardless of url', () => {
    expect(resolveAppTarget({ app: 'teams' }, 'https://anything.example', primary, true))
      .toEqual({ launch: true, reason: null, fallback: primary })
  })

  it('teams falls back when not installed, with no convertibility check at all', () => {
    expect(resolveAppTarget({ app: 'teams' }, 'https://anything.example', primary, false))
      .toEqual({ launch: false, reason: 'app-not-installed', fallback: primary })
  })

  it('zoom launches when installed and the url converts', () => {
    expect(resolveAppTarget({ app: 'zoom' }, 'https://zoom.us/j/123456789', primary, true))
      .toEqual({ launch: true, reason: null, fallback: primary })
  })

  it('zoom falls back with "not-convertible" before even checking installed', () => {
    expect(resolveAppTarget({ app: 'zoom' }, 'https://zoom.us/pricing', primary, true))
      .toEqual({ launch: false, reason: 'not-convertible', fallback: primary })
    expect(resolveAppTarget({ app: 'zoom' }, 'https://zoom.us/pricing', primary, false))
      .toEqual({ launch: false, reason: 'not-convertible', fallback: primary })
  })

  it('zoom falls back with "app-not-installed" when the link is convertible but the app is missing', () => {
    expect(resolveAppTarget({ app: 'zoom' }, 'https://zoom.us/j/123456789', primary, false))
      .toEqual({ launch: false, reason: 'app-not-installed', fallback: primary })
  })

  it('uses the rule\'s own fallback over primary when set', () => {
    const fallback = { browser: 'chrome', private: false }
    expect(resolveAppTarget({ app: 'teams', fallback }, 'https://x.example', primary, false))
      .toEqual({ launch: false, reason: 'app-not-installed', fallback })
  })

  it('with no url context (e.g. the static Routes table), zoom is assumed convertible', () => {
    expect(resolveAppTarget({ app: 'zoom' }, null, primary, true))
      .toEqual({ launch: true, reason: null, fallback: primary })
    expect(resolveAppTarget({ app: 'zoom' }, null, primary, false))
      .toEqual({ launch: false, reason: 'app-not-installed', fallback: primary })
  })
})
