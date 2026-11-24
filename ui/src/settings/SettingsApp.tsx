import { useEffect, useMemo, useState } from 'react'
import { getConfig, getPlatformCaps, isDefaultBrowser, listBrowsers, listPresets, saveConfig, setDefaultBrowser } from '../api'
import type { Browser, Config, OpenIn, PlatformCaps, Preset, Rule } from '../api'
import { BrowserIcon } from '../BrowserIcon'
import { deriveTargets, isAppTarget, targetKey, visibleTargets } from '../targets'
import type { BrowserTarget } from '../targets'
import { parseTraceUrl, traceRoute } from '../routing'
import type { Trace } from '../routing'

interface RuleDraft {
  index: number | null
  name: string
  patterns: string
  sourceApps: string
  openIn: string
  fallback: string
  preset?: Rule['preset']
}

const emptyDraft: RuleDraft = { index: null, name: '', patterns: '', sourceApps: '', openIn: 'prompt', fallback: '' }

function lines(value: string): string[] {
  return value.split('\n').map((line) => line.trim()).filter(Boolean)
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason)
}

function targetOptions(targets: BrowserTarget[]) {
  return targets.map(({ key, label }) => <option key={key} value={key}>{label}</option>)
}

function presetId(id: string): Rule['preset'] {
  return id === 'meet' || id === 'teams' || id === 'slack' || id === 'zoom' ? id : undefined
}

function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

function capitalize(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1)
}

function appLabel(app: 'teams' | 'zoom'): string {
  return app === 'teams' ? 'Teams' : 'Zoom'
}

/** Mirrors the real routing decision's destination for display, so a
 * rule's "Departs to" never disagrees with what it actually does. */
function destinationLabel(openIn: OpenIn, targets: BrowserTarget[]): string {
  if (openIn === 'prompt') return 'Ask (picker)'
  if (isAppTarget(openIn)) return appLabel(openIn.app)
  return targets.find(({ key }) => key === targetKey(openIn))?.label ?? targetKey(openIn)
}

function CheckIcon() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M13.5 4.5 6 12 2.5 8.5l1-1L6 10l6.5-6.5z" /></svg>
  )
}

export function SettingsApp() {
  const [config, setConfig] = useState<Config | null>(null)
  const [browsers, setBrowsers] = useState<Browser[]>([])
  const [caps, setCaps] = useState<PlatformCaps | null>(null)
  const [defaultBrowser, setIsDefault] = useState<boolean | null>(null)
  const [defaultMessage, setDefaultMessage] = useState('')
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)
  const [busy, setBusy] = useState(false)
  const [rescanning, setRescanning] = useState(false)
  const [draft, setDraft] = useState<RuleDraft | null>(null)
  const [section, setSection] = useState<'general' | 'browsers' | 'rules'>('general')
  const [presets, setPresets] = useState<Preset[]>([])
  const [presetMenu, setPresetMenu] = useState(false)
  const [traceInput, setTraceInput] = useState('')
  const targets = useMemo(() => deriveTargets(browsers), [browsers])

  async function reload() {
    const [nextConfig, nextBrowsers, nextCaps, isDefault] = await Promise.all([
      getConfig(), listBrowsers(), getPlatformCaps(), isDefaultBrowser(),
    ])
    setConfig(nextConfig)
    setBrowsers(nextBrowsers)
    setCaps(nextCaps)
    setIsDefault(isDefault)
  }

  useEffect(() => {
    let mounted = true
    Promise.all([getConfig(), listBrowsers(), getPlatformCaps(), isDefaultBrowser()])
      .then(([nextConfig, nextBrowsers, nextCaps, isDefault]) => {
        if (!mounted) return
        setConfig(nextConfig)
        setBrowsers(nextBrowsers)
        setCaps(nextCaps)
        setIsDefault(isDefault)
      })
      .catch((reason: unknown) => { if (mounted) setError(errorMessage(reason)) })
    return () => { mounted = false }
  }, [])

  useEffect(() => {
    if (section !== 'rules') return
    let active = true
    listPresets().then((items) => { if (active) setPresets(items) })
      .catch((reason: unknown) => { if (active) setError(errorMessage(reason)) })
    return () => { active = false }
  }, [section])

  function updateConfig(patch: Partial<Config>) {
    setConfig((current) => current ? { ...current, ...patch } : current)
    setSaved(false)
  }

  function updateRules(rules: Rule[]) {
    updateConfig({ rules })
  }

  function editRule(index: number) {
    if (!config) return
    const rule = config.rules[index]
    setDraft({
      index, name: rule.name, patterns: rule.match.join('\n'),
      sourceApps: rule.source_apps?.join('\n') ?? '',
      openIn: rule.open_in === 'prompt' ? 'prompt' : isAppTarget(rule.open_in) ? `app:${rule.open_in.app}` : targetKey(rule.open_in),
      fallback: isAppTarget(rule.open_in) && rule.open_in.fallback ? targetKey(rule.open_in.fallback) : '',
      preset: rule.preset,
    })
    setSection('rules')
  }

  function addPreset(preset: Preset) {
    const id = presetId(preset.id)
    if (!id) return
    const app = preset.app === 'teams' || preset.app === 'zoom' ? preset.app : null
    setDraft({
      ...emptyDraft, name: preset.name, patterns: preset.patterns.join('\n'), preset: id,
      openIn: app ? `app:${app}` : config?.primary ? targetKey(config.primary) : 'prompt',
      fallback: app && config?.primary ? targetKey(config.primary) : '',
    })
    setPresetMenu(false)
    setSection('rules')
  }

  function commitRule() {
    if (!config || !draft) return
    const name = draft.name.trim()
    const match = lines(draft.patterns)
    if (!name || match.length === 0) {
      setError('Enter a rule name and at least one URL pattern.')
      return
    }
    const previous = draft.index === null ? undefined : config.rules[draft.index]
    const previousTarget = previous?.open_in !== 'prompt' && previous?.open_in && !isAppTarget(previous.open_in)
      ? previous.open_in : undefined
    const selectedTarget = targets.find(({ key }) => key === draft.openIn)?.target
      ?? (targetKey(config.primary) === draft.openIn ? config.primary : undefined)
      ?? (previousTarget && targetKey(previousTarget) === draft.openIn ? previousTarget : undefined)
    const previousFallback = previous?.open_in !== 'prompt' && previous?.open_in && isAppTarget(previous.open_in)
      ? previous.open_in.fallback : undefined
    const fallback = targets.find(({ key }) => key === draft.fallback)?.target
      ?? (targetKey(config.primary) === draft.fallback ? config.primary : undefined)
      ?? (previousFallback && targetKey(previousFallback) === draft.fallback ? previousFallback : undefined)
    const openIn: OpenIn = draft.openIn === 'app:teams' || draft.openIn === 'app:zoom'
      ? { app: draft.openIn.slice(4) as 'teams' | 'zoom', ...(fallback ? { fallback } : {}) }
      : draft.openIn === 'prompt' ? 'prompt' : selectedTarget ?? 'prompt'
    const rule: Rule = { ...previous, name, match, open_in: openIn }
    if (draft.preset) rule.preset = draft.preset
    if (caps?.source_app_rules) {
      const sourceApps = lines(draft.sourceApps)
      if (sourceApps.length) rule.source_apps = sourceApps
      else delete rule.source_apps
    }
    const rules = [...config.rules]
    if (draft.index === null) rules.push(rule)
    else rules[draft.index] = rule
    updateRules(rules)
    setDraft(null)
    setError('')
  }

  function moveRule(index: number, offset: number) {
    if (!config) return
    const next = index + offset
    if (next < 0 || next >= config.rules.length) return
    const rules = [...config.rules]
    ;[rules[index], rules[next]] = [rules[next], rules[index]]
    updateRules(rules)
  }

  async function makeDefault() {
    setBusy(true)
    setError('')
    setDefaultMessage('')
    try {
      await setDefaultBrowser()
      const isDefault = await isDefaultBrowser()
      setIsDefault(isDefault)
      if (isDefault) setDefaultMessage('Linh is your default browser.')
      else setError('Linh is not the default browser yet. Check your system settings and try again.')
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setBusy(false)
    }
  }

  async function save() {
    if (!config || busy) return
    setBusy(true)
    setError('')
    setSaved(false)
    try {
      await saveConfig(config)
      await reload()
      setSaved(true)
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setBusy(false)
    }
  }

  async function rescan() {
    if (rescanning) return
    setRescanning(true)
    setError('')
    try {
      await reload()
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setRescanning(false)
    }
  }

  const primaryKey = config ? targetKey(config.primary) : ''
  const knownPrimary = targets.some(({ key }) => key === primaryKey)
  const openInKnown = draft?.openIn === 'prompt' || draft?.openIn.startsWith('app:') || targets.some(({ key }) => key === draft?.openIn)
  const fallbackKnown = !draft?.fallback || targets.some(({ key }) => key === draft.fallback)
  const appPreset = presets.find(({ app }) => app === draft?.openIn.slice(4))
  const staleShownBrowsers = config
    ? config.shown_browsers.filter((key) => !targets.some((target) => target.key === key))
    : []
  const primaryLabel = config ? targets.find(({ key }) => key === primaryKey)?.label ?? primaryKey : ''
  const visible = config ? visibleTargets(targets, config.shown_browsers) : []

  return (
    <main className="settings">
      <div className="settings-topbar">
        <div className="settings-logo"><strong>LINH</strong><span>/ link dispatch</span></div>
        <nav className="settings-tabs" aria-label="Settings sections">
          {(['general', 'browsers', 'rules'] as const).map((item) =>
            <button key={item} type="button" aria-current={section === item ? 'page' : undefined} onClick={() => setSection(item)}>
              {section === item && <span className="nav-pip" aria-hidden="true" />}
              {item[0].toUpperCase() + item.slice(1)}
            </button>)}
        </nav>
      </div>
      {defaultMessage && <p role="status" className="success" style={{ padding: '0 28px' }}>{defaultMessage}</p>}
      {error && <p role="alert" className="error" style={{ padding: '0 28px' }}>{error}</p>}
      {!config && !error && <p style={{ padding: '32px 28px' }}>Loading settings…</p>}
      {config && caps && <div className="settings-body">
        <div className="settings-board">
          {section === 'general' && defaultBrowser === false && <section className="banner" aria-label="Default browser setup">
            <h2>Make Linh your default browser</h2>
            <p>Let Linh receive links from other apps so it can apply your rules.</p>
            <button type="button" disabled={busy} onClick={() => void makeDefault()}>Make Linh your default browser</button>
          </section>}
          {section === 'general' && <>
            <div className="settings-heading">
              <div><h2>Defaults</h2><p>What happens when a link matches no route.</p></div>
            </div>
            <div>
              <div className="settings-table-head"><span style={{ flex: '1 1 0' }}>Setting</span><span style={{ flex: '0 0 260px' }}>Value</span></div>
              <ul className="settings-rows">
                <li className="settings-row">
                  <div className="col-main"><label htmlFor="primary" className="primary" style={{ margin: 0 }}>Primary browser</label><p className="secondary" style={{ margin: 0 }}>Used when a link opens without asking.</p></div>
                  <div className="target-select" style={{ flex: '0 0 260px' }}>
                    <BrowserIcon icon={targets.find(({ key }) => key === primaryKey)?.icon} />
                    <select id="primary" value={primaryKey} onChange={(event) => {
                      const selected = targets.find(({ key }) => key === event.target.value)
                      if (selected) updateConfig({ primary: selected.target })
                    }}>
                      {!knownPrimary && <option value={primaryKey}>Current browser (not detected)</option>}
                      {targetOptions(targets)}
                    </select>
                  </div>
                </li>
                <li className="settings-row">
                  <div className="col-main"><label htmlFor="prompt-mode" className="primary" style={{ margin: 0 }}>Prompt mode</label><p className="secondary" style={{ margin: 0 }}>Always shows the picker for every link.</p></div>
                  <select id="prompt-mode" style={{ flex: '0 0 260px' }} value={config.prompt_mode} onChange={(event) => updateConfig({ prompt_mode: event.target.value as Config['prompt_mode'] })}>
                    <option value="never">Never</option>
                    <option value="always">Always</option>
                  </select>
                </li>
              </ul>
            </div>
            <p className="note hint">Linux has no modifier-key prompt mode.</p>
          </>}
          {section === 'browsers' && <>
            <div className="settings-heading">
              <div><h2>Targets</h2><p>Choose which browsers appear in the picker. None selected shows all.</p></div>
              <div className="rule-actions"><button type="button" className="secondary" disabled={rescanning} onClick={() => void rescan()}>Rescan</button></div>
            </div>
            <div>
              <div className="settings-table-head">
                <span className="col-no">No</span><span style={{ flex: '1 1 0' }}>Target</span><span className="col-private">Private</span><span className="col-pick">In picker</span>
              </div>
              <ul className="checklist settings-rows" aria-label="Shown browsers">
                {targets.map(({ key, label, icon, target }, index) => {
                  const supportsPrivate = browsers.find((browser) => browser.id === target.browser)?.supports_private
                  const checked = config.shown_browsers.includes(key)
                  return (
                    <li key={key}>
                      <label className={`settings-row${checked ? ' selected' : ''}`}>
                        <span className="col-no">{pad2(index + 1)}</span>
                        <span className="col-name"><BrowserIcon icon={icon} />{label}</span>
                        <span className="col-private">{supportsPrivate ? 'available' : '—'}</span>
                        <span className="col-pick">
                          <input className="sr-only-input" type="checkbox" aria-label={`Show ${label}`} checked={checked} onChange={(event) => {
                            updateConfig({ shown_browsers: event.target.checked
                              ? [...config.shown_browsers, key]
                              : config.shown_browsers.filter((item) => item !== key) })
                          }} />
                          <span className={`check-box${checked ? ' checked' : ''}`} aria-hidden="true">{checked && <CheckIcon />}</span>
                        </span>
                      </label>
                    </li>
                  )
                })}
                {targets.length === 0 && <p>No browsers detected.</p>}
                {staleShownBrowsers.map((key) => <li key={key}>
                  <label className="settings-row selected">
                    <span className="col-no">—</span>
                    <span className="col-name">{`Not installed: ${key}`}</span>
                    <span className="col-private">—</span>
                    <span className="col-pick">
                      <input className="sr-only-input" type="checkbox" aria-label={`Show Not installed: ${key}`} checked onChange={() => {
                        updateConfig({ shown_browsers: config.shown_browsers.filter((item) => item !== key) })
                      }} />
                      <span className="check-box checked" aria-hidden="true"><CheckIcon /></span>
                    </span>
                  </label>
                </li>)}
              </ul>
            </div>
          </>}
          {section === 'rules' && <>
            <div className="settings-heading">
              <div><h2>Routes</h2><p>Every link is checked against routes in order. First match departs.</p></div>
              <div className="rule-actions"><button type="button" className="secondary" aria-expanded={presetMenu} onClick={() => setPresetMenu(!presetMenu)}>Presets</button><button type="button" className="primary" onClick={() => setDraft({ ...emptyDraft })}>+ New route</button></div>
            </div>
            {presetMenu && <div className="preset-menu" role="menu" aria-label="Presets">{presets.map((preset) => <button type="button" role="menuitem" key={preset.id} onClick={() => addPreset(preset)}>{preset.name}</button>)}</div>}
            {!caps.source_app_rules && <p className="hint">Source app rules are unavailable on Linux. Existing source app values are kept when you save.</p>}
            <div>
              <div className="settings-table-head">
                <span className="col-no">No</span><span style={{ flex: '1 1 0' }}>Route / matches</span><span className="col-dest">Departs to</span>
              </div>
              <ol aria-label="Routes" className="settings-rows rules">
                {config.rules.map((rule, index) => <li key={index} className="settings-row">
                  <span className="col-no">{pad2(index + 1)}</span>
                  <div className="col-main">
                    <h3 className="primary" style={{ margin: 0, fontWeight: 500, fontSize: 14 }}>{rule.name}{rule.preset && <> <span className="preset-badge">{rule.preset}</span></>}</h3>
                    <p className="secondary" style={{ margin: 0 }}>{rule.match[0]}{rule.match.length > 1 && `  +${rule.match.length - 1}`}</p>
                  </div>
                  <span className="col-dest">{destinationLabel(rule.open_in, targets)}</span>
                  <div className="rule-actions">
                    <button type="button" className="secondary" aria-label={`Move ${rule.name} up`} disabled={index === 0} onClick={() => moveRule(index, -1)}>↑</button>
                    <button type="button" className="secondary" aria-label={`Move ${rule.name} down`} disabled={index === config.rules.length - 1} onClick={() => moveRule(index, 1)}>↓</button>
                    <button type="button" className="secondary" aria-label={`Edit ${rule.name}`} onClick={() => editRule(index)}>Edit</button>
                    <button type="button" className="secondary" aria-label={`Delete ${rule.name}`} onClick={() => updateRules(config.rules.filter((_, i) => i !== index))}>Delete</button>
                  </div>
                </li>)}
                <li className="settings-row">
                  <span className="col-no">—</span>
                  <div className="col-main"><p className="primary" style={{ margin: 0 }}>No match</p><p className="secondary" style={{ margin: 0 }}>any other link</p></div>
                  <span className="col-dest">{config.prompt_mode === 'always' ? 'Ask (picker)' : destinationLabel(config.primary, targets)}</span>
                </li>
              </ol>
            </div>
            {draft && <div className="rule-editor">
              <h3>{draft.index === null ? 'Add rule' : 'Edit rule'}</h3>
              <label htmlFor="rule-name">Rule name</label>
              <input id="rule-name" value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
              <label htmlFor="rule-patterns">URL patterns</label>
              <textarea id="rule-patterns" rows={4} placeholder={'meet.google.com\n*.company.com'} value={draft.patterns} onChange={(event) => setDraft({ ...draft, patterns: event.target.value })} />
              {caps.source_app_rules && <><label htmlFor="source-apps">Source apps</label><textarea id="source-apps" rows={2} value={draft.sourceApps} onChange={(event) => setDraft({ ...draft, sourceApps: event.target.value })} /></>}
              <label htmlFor="rule-open-in">Open in</label>
              <div className="target-select"><BrowserIcon icon={targets.find(({ key }) => key === draft.openIn)?.icon} />
              <select id="rule-open-in" value={draft.openIn} onChange={(event) => setDraft({ ...draft, openIn: event.target.value })}>
                <option value="prompt">Prompt</option>
                <option value="app:teams">Teams app</option>
                <option value="app:zoom">Zoom app</option>
                {!openInKnown && <option value={draft.openIn}>Current target (not detected)</option>}
                {targetOptions(targets)}
              </select></div>
              {draft.openIn.startsWith('app:') && <>
                <p className="app-status">{appPreset
                  ? (appPreset.app_installed ? 'App installed' : 'App not found — links will open in the fallback browser')
                  : 'App availability unavailable'}</p>
                <label htmlFor="rule-fallback">Fallback browser</label>
                <div className="target-select"><BrowserIcon icon={targets.find(({ key }) => key === draft.fallback)?.icon} />
                  <select id="rule-fallback" value={draft.fallback} onChange={(event) => setDraft({ ...draft, fallback: event.target.value })}>
                    <option value="">Primary browser</option>
                    {!fallbackKnown && <option value={draft.fallback}>Current target (not detected)</option>}
                    {targetOptions(targets)}
                  </select>
                </div>
              </>}
              <div className="editor-actions"><button type="button" className="primary-action" onClick={commitRule}>Save rule</button><button type="button" className="secondary" onClick={() => setDraft(null)}>Cancel</button></div>
            </div>}
          </>}
        </div>
        <SidePanel section={section} config={config} caps={caps} targets={targets} primaryLabel={primaryLabel} visible={visible}
          traceInput={traceInput} onTraceInputChange={setTraceInput} />
      </div>}
      <footer className="settings-footer"><button type="button" className="primary-action" disabled={busy} onClick={() => void save()}>Save settings</button>{saved && <span role="status">Settings saved.</span>}</footer>
    </main>
  )
}

interface SidePanelProps {
  section: 'general' | 'browsers' | 'rules'
  config: Config
  caps: PlatformCaps
  targets: BrowserTarget[]
  primaryLabel: string
  visible: BrowserTarget[]
  traceInput: string
  onTraceInputChange: (value: string) => void
}

/** Every number here is derived live from `config`/`targets`/`caps` (the
 * same data the board already fetched) — never a static mock — so it can
 * never show a destination the real router wouldn't actually pick. */
function SidePanel({ section, config, caps, targets, primaryLabel, visible, traceInput, onTraceInputChange }: SidePanelProps) {
  if (section === 'general') {
    const keysShown = Math.min(visible.length, 9)
    return (
      <aside className="settings-side">
        <p className="eyebrow">Current behaviour</p>
        <div className="settings-side-flow">
          <div className="settings-side-step"><span className="step-no">01</span><span className="step-name">Link clicked</span><span className="step-tag">any app</span></div>
          <div className="settings-side-step"><span className="step-no">02</span><span className="step-name">Check routes</span><span className="step-tag">{config.rules.length} routes</span></div>
          <div className="settings-side-step"><span className="step-no">03</span><span className="step-name">No match</span><span className="step-tag">prompt: {capitalize(config.prompt_mode)}</span></div>
        </div>
        <div className="settings-side-spacer" />
        <div className="settings-side-result">
          <p className="eyebrow">Departs to</p>
          {config.prompt_mode === 'always'
            ? <><p className="value">Picker</p><p className="caption">{visible.length} targets shown · keys 1–{keysShown}</p></>
            : <><p className="value">{primaryLabel}</p><p className="caption">opens directly · no prompt</p></>}
        </div>
      </aside>
    )
  }

  if (section === 'browsers') {
    const keysShown = Math.min(visible.length, 9)
    return (
      <aside className="settings-side">
        <p className="eyebrow">Picker preview</p>
        <div className="settings-side-flow">
          {visible.map((target, index) => (
            <div className="settings-side-step" key={target.key}>
              <span className="step-no">{index + 1}</span><span className="step-name">{target.label}</span>
            </div>
          ))}
          {visible.length === 0 && <p className="settings-side-note">No targets shown.</p>}
        </div>
        <div className="settings-side-spacer" />
        <div className="settings-side-result">
          <p className="eyebrow">Shown</p>
          <p className="value">{visible.length} of {targets.length}</p>
          <p className="caption">keys 1–{keysShown} open directly</p>
        </div>
      </aside>
    )
  }

  const parsed = parseTraceUrl(traceInput)
  const trace: Trace | null = parsed ? traceRoute(config, caps, parsed.host, parsed.path) : null

  return (
    <aside className="settings-side">
      <p className="eyebrow">Test a link</p>
      <div className="settings-side-input">
        <input
          aria-label="Test a link"
          placeholder="github.com/owner/repo"
          value={traceInput}
          onChange={(event) => onTraceInputChange(event.target.value)}
        />
      </div>
      {!trace && <p className="settings-side-note">{traceInput.trim() ? 'Enter a valid URL.' : 'Enter a URL to see which route it takes.'}</p>}
      {trace && <>
        <div className="settings-side-flow">
          {trace.decision.kind === 'prompt-always' && <p className="settings-side-note">Prompt mode is Always — every link opens the picker, routes are not checked.</p>}
          {config.rules.length === 0 && trace.decision.kind !== 'prompt-always' && <p className="settings-side-note">No routes defined.</p>}
          {trace.steps.map((step) => (
            <div className={`settings-side-step${step.status === 'match' ? ' match' : ''}`} key={step.index}>
              <span className="step-no">{pad2(step.index + 1)}</span>
              <span className="step-name">{step.name}</span>
              <span className="step-tag">{step.status === 'match' ? 'match' : 'skip'}</span>
            </div>
          ))}
        </div>
        <div className="settings-side-spacer" />
        <div className="settings-side-result">
          <p className="eyebrow">Departs to</p>
          {(() => {
            const d = trace.decision
            if (d.kind === 'prompt-always' || d.kind === 'prompt') return <p className="value">Picker</p>
            if (d.kind === 'open-app') return <p className="value">{appLabel(d.app.app)}</p>
            return <p className="value">{targets.find(({ key }) => key === targetKey(d.target))?.label ?? targetKey(d.target)}</p>
          })()}
          <p className="caption">
            {trace.decision.kind === 'prompt-always'
              ? 'prompt mode: always'
              : trace.matchedIndex !== null
                ? `via route ${pad2(trace.matchedIndex + 1)} · ${parsed?.host}`
                : `no match · primary browser`}
          </p>
        </div>
      </>}
    </aside>
  )
}
