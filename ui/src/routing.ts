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

/** Parses user input for the test-a-link box into a host/path pair,
 * assuming https when no scheme was typed (matching how a bare domain
 * typed by a person is meant). Returns null for empty/unparsable input. */
export function parseTraceUrl(raw: string): { host: string; path: string } | null {
  const trimmed = raw.trim()
  if (!trimmed) return null
  const withScheme = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(trimmed) ? trimmed : `https://${trimmed}`
  try {
    const url = new URL(withScheme)
    return { host: url.host, path: url.pathname }
  } catch {
    return null
  }
}

/** Ports routing.rs::decide, but reports every step along the way (not
 * just the final decision) so the UI can render the full trace. */
export function traceRoute(config: Config, caps: PlatformCaps, host: string, path: string): Trace {
  if (config.prompt_mode === 'always') {
    return { steps: [], decision: { kind: 'prompt-always' }, matchedIndex: null }
  }

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
  return { steps, decision: decision ?? { kind: 'open', target: config.primary }, matchedIndex }
}

function openInToDecision(openIn: OpenIn): TraceDecision {
  if (openIn === 'prompt') return { kind: 'prompt' }
  if (isAppTarget(openIn)) return { kind: 'open-app', app: openIn }
  return { kind: 'open', target: openIn }
}
