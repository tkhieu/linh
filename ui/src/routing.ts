// Ports crates/linh-core/src/matching.rs and routing.rs so the Settings
// "test a link" preview traces rules with the exact same semantics as the
// real router, instead of risking disagreement with a second, divergent
// implementation.
import type { AppTarget, Config, OpenIn, PlatformCaps, Target } from './api'
import { isAppTarget } from './targets'

/** Minimal glob matcher supporting `*` (matches any sequence of
 * characters, including none). Anchored at both ends. Ports the
 * dynamic-programming matcher in matching.rs::glob_match. */
function globMatch(pattern: string, text: string): boolean {
  const p = Array.from(pattern)
  const t = Array.from(text)
  const dp: boolean[][] = Array.from({ length: p.length + 1 }, () => new Array<boolean>(t.length + 1).fill(false))
  dp[0][0] = true
  for (let i = 1; i <= p.length; i++) {
    if (p[i - 1] === '*') dp[i][0] = dp[i - 1][0]
  }
  for (let i = 1; i <= p.length; i++) {
    for (let j = 1; j <= t.length; j++) {
      dp[i][j] = p[i - 1] === '*' ? dp[i - 1][j] || dp[i][j - 1] : dp[i - 1][j - 1] && p[i - 1] === t[j - 1]
    }
  }
  return dp[p.length][t.length]
}

function domainMatches(domainPattern: string, host: string): boolean {
  if (domainPattern.includes('*')) return globMatch(domainPattern, host)
  return host === domainPattern || host.endsWith(`.${domainPattern}`)
}

/** Ports matching.rs::pattern_matches. */
export function patternMatches(pattern: string, host: string, path: string): boolean {
  const loweredPattern = pattern.toLowerCase()
  const loweredHost = host.toLowerCase()
  const slashAt = loweredPattern.indexOf('/')
  const domainPattern = slashAt === -1 ? loweredPattern : loweredPattern.slice(0, slashAt)
  const pathPattern = slashAt === -1 ? null : loweredPattern.slice(slashAt + 1)

  if (!domainMatches(domainPattern, loweredHost)) return false
  if (pathPattern === null) return true

  const actualPath = path.replace(/^\/+/, '')
  const pp = pathPattern.replace(/^\/+/, '')
  return globMatch(pp, actualPath)
}

export type TraceStatus = 'match' | 'skip' | 'skip-no-source-app-caps'

export interface TraceStep {
  index: number
  name: string
  status: TraceStatus
}

export type TraceDecision =
  | { kind: 'prompt-always' }
  | { kind: 'prompt' }
  | { kind: 'open'; target: Target }
  | { kind: 'open-app'; app: AppTarget }

export interface Trace {
  steps: TraceStep[]
  decision: TraceDecision
  matchedIndex: number | null
}

/** Parses user input for the test-a-link box exactly like
 * `url_intake::filter_routable_url`: must parse as a URL and have an
 * `http`/`https` scheme, or it's not routable at all — never prefixed
 * with `https://` (that function never does either), and `host` is the
 * hostname only (no port), matching `Url::host_str()`. Returns null for
 * empty input or anything not routable. */
export function parseTraceUrl(raw: string): { host: string; path: string; href: string } | null {
  const trimmed = raw.trim()
  if (!trimmed) return null
  let url: URL
  try {
    url = new URL(trimmed)
  } catch {
    return null
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return null
  return { host: url.hostname, path: url.pathname, href: url.href }
}

/** Ports routing.rs::decide, but reports every step along the way (not
 * just the final decision) so the UI can render the full trace. */
export function traceRoute(config: Config, caps: PlatformCaps, host: string, path: string): Trace {
  // Every rule gets exactly one step, even past the winner: once a rule
  // matches, routing.rs::decide returns immediately without evaluating
  // the rest, so later rules are unreachable rather than independently
  // "not matching" — 'skip' reports that honestly instead of implying
  // they were checked.
  const steps: TraceStep[] = []
  let decision: TraceDecision | null = null
  let matchedIndex: number | null = null
  for (let index = 0; index < config.rules.length; index++) {
    const rule = config.rules[index]
    if (decision) {
      steps.push({ index, name: rule.name, status: 'skip' })
      continue
    }
    if (rule.source_apps && !caps.source_app_rules) {
      steps.push({ index, name: rule.name, status: 'skip-no-source-app-caps' })
      continue
    }
    const matched = rule.match.some((pattern) => patternMatches(pattern, host, path))
    if (matched) {
      steps.push({ index, name: rule.name, status: 'match' })
      decision = openInToDecision(rule.open_in)
      matchedIndex = index
    } else {
      steps.push({ index, name: rule.name, status: 'skip' })
    }
  }
  return { steps, decision: decision ?? (config.prompt_mode === 'always'
    ? { kind: 'prompt-always' }
    : { kind: 'open', target: config.primary }), matchedIndex }
}

function openInToDecision(openIn: OpenIn): TraceDecision {
  if (openIn === 'prompt') return { kind: 'prompt' }
  if (isAppTarget(openIn)) return { kind: 'open-app', app: openIn }
  return { kind: 'open', target: openIn }
}

/** Ports zoom.rs::zoom_app_url, byte-for-byte: a path-segment split that
 * mirrors `Url::path_segments()` (the path of an http(s) URL always
 * starts with '/', so splitting everything after that first slash on
 * '/' reproduces its iterator, empty trailing segments included), and a
 * raw (non-percent-decoded) scan of the query string for `pwd=...` so
 * its value is carried over byte-for-byte rather than risking a
 * re-encoding mismatch — exactly the reason the Rust side avoids
 * `query_pairs()` too. */
export function zoomAppUrl(rawUrl: string): string | null {
  let url: URL
  try {
    url = new URL(rawUrl)
  } catch {
    return null
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return null

  const host = url.hostname
  if (host !== 'zoom.us' && !host.endsWith('.zoom.us')) return null

  const segments = url.pathname.slice(1).split('/')
  if (segments[0] !== 'j') return null
  const id = segments[1]
  if (id === undefined || segments.length > 2) return null
  if (id.length === 0 || !/^[0-9]+$/.test(id)) return null

  const query = url.search.startsWith('?') ? url.search.slice(1) : null
  const pwd = query?.split('&').find((pair) => pair.startsWith('pwd=' ))?.slice(4) ?? null

  return `zoommtg://${host}/join?action=join&confno=${id}${pwd !== null ? `&pwd=${pwd}` : ''}`
}

export type AppDecisionReason = 'app-not-installed' | 'not-convertible'

/** Whether `app_installed` for an app is known yet: `listPresets()`
 * starts empty and resolves asynchronously, and can fail — "not yet
 * loaded" and "failed to load" both mean the same thing here, that we
 * genuinely don't know, not that the app is missing. */
export type AppAvailability = 'installed' | 'missing' | 'unknown'

export interface AppDecision {
  /** `'unknown'` means exactly that — availability hasn't been
   * determined, so this is not a claim that it will or won't launch. */
  launch: boolean | 'unknown'
  reason: AppDecisionReason | null
  fallback: Target
}

/** Ports src-tauri/src/routing.rs::plan_app_open's pure decision (never
 * the actual spawn): whether `appTarget`'s app would actually launch for
 * `url`, and if not, why and what it falls back to. `url` is `null` when
 * there is no specific link to test against (e.g. the static Routes
 * table) — Teams has no URL-convertibility check at all (it always
 * passes the raw URL through), and Zoom's is skipped the same way the
 * Rust side would have nothing to check against. Checks convertibility
 * before availability, exactly like `plan_app_open`'s order: a
 * not-convertible Zoom link falls back regardless of whether the app's
 * installed state is even known yet — that part is a pure fact about
 * the URL, not a guess. Only once convertibility is settled does an
 * `'unknown'` availability itself become the answer, rather than
 * guessing `true` or `false` for it. */
export function resolveAppTarget(appTarget: AppTarget, url: string | null, primary: Target, availability: AppAvailability): AppDecision {
  const fallback = appTarget.fallback ?? primary
  if (appTarget.app === 'zoom' && url !== null && zoomAppUrl(url) === null) {
    return { launch: false, reason: 'not-convertible', fallback }
  }
  if (availability === 'unknown') return { launch: 'unknown', reason: null, fallback }
  if (availability === 'missing') return { launch: false, reason: 'app-not-installed', fallback }
  return { launch: true, reason: null, fallback }
}
