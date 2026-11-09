import { useEffect, useMemo, useState } from 'react'
import { getConfig, getPlatformCaps, isDefaultBrowser, listBrowsers, listPresets, saveConfig, setDefaultBrowser } from '../api'
import type { Browser, Config, OpenIn, PlatformCaps, Preset, Rule } from '../api'
import { BrowserIcon } from '../BrowserIcon'
import { deriveTargets, isAppTarget, targetKey } from '../targets'
import type { BrowserTarget } from '../targets'

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

export function SettingsApp() {
  const [config, setConfig] = useState<Config | null>(null)
  const [browsers, setBrowsers] = useState<Browser[]>([])
  const [caps, setCaps] = useState<PlatformCaps | null>(null)
  const [defaultBrowser, setIsDefault] = useState<boolean | null>(null)
  const [defaultMessage, setDefaultMessage] = useState('')
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)
  const [busy, setBusy] = useState(false)
  const [draft, setDraft] = useState<RuleDraft | null>(null)
  const [section, setSection] = useState<'general' | 'browsers' | 'rules'>('general')
  const [presets, setPresets] = useState<Preset[]>([])
  const [presetMenu, setPresetMenu] = useState(false)
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

  const primaryKey = config ? targetKey(config.primary) : ''
  const knownPrimary = targets.some(({ key }) => key === primaryKey)
  const openInKnown = draft?.openIn === 'prompt' || draft?.openIn.startsWith('app:') || targets.some(({ key }) => key === draft?.openIn)
  const fallbackKnown = !draft?.fallback || targets.some(({ key }) => key === draft.fallback)
  const appPreset = presets.find(({ app }) => app === draft?.openIn.slice(4))
  const staleShownBrowsers = config
    ? config.shown_browsers.filter((key) => !targets.some((target) => target.key === key))
    : []

  return (
    <main className="settings">
      <header><h1>Linh Settings</h1><p>Choose where links open.</p></header>
      <nav className="settings-tabs" aria-label="Settings sections">
        {(['general', 'browsers', 'rules'] as const).map((item) =>
          <button key={item} type="button" aria-current={section === item ? 'page' : undefined} onClick={() => setSection(item)}>
            {item[0].toUpperCase() + item.slice(1)}
          </button>)}
      </nav>
      {section === 'general' && defaultBrowser === false && <section className="banner" aria-label="Default browser setup">
        <h2>Make Linh your default browser</h2>
        <p>Let Linh receive links from other apps so it can apply your rules.</p>
        <button type="button" disabled={busy} onClick={() => void makeDefault()}>Make Linh your default browser</button>
      </section>}
      {defaultMessage && <p role="status" className="success">{defaultMessage}</p>}
      {error && <p role="alert" className="error">{error}</p>}
      {config && caps && <>
        {section === 'general' && <section>
          <h2>General</h2>
          <label htmlFor="primary">Primary browser</label>
          <div className="target-select"><BrowserIcon icon={targets.find(({ key }) => key === primaryKey)?.icon} />
          <select id="primary" value={primaryKey} onChange={(event) => {
            const selected = targets.find(({ key }) => key === event.target.value)
            if (selected) updateConfig({ primary: selected.target })
          }}>
            {!knownPrimary && <option value={primaryKey}>Current browser (not detected)</option>}
            {targetOptions(targets)}
          </select></div>
          <label htmlFor="prompt-mode">Prompt mode</label>
          <select id="prompt-mode" value={config.prompt_mode} onChange={(event) => updateConfig({ prompt_mode: event.target.value as Config['prompt_mode'] })}>
            <option value="never">Never</option>
            <option value="always">Always</option>
          </select>
          <p className="hint">Always shows the picker for every link. Linux has no modifier key mode.</p>
        </section>}
        {section === 'browsers' && <section>
          <h2>Shown browsers</h2>
          <p className="hint">Select the targets to show in the picker. No selection shows all targets.</p>
          <div className="checklist">
            {targets.map(({ key, label, icon }) => <label key={key}>
              <input type="checkbox" aria-label={`Show ${label}`} checked={config.shown_browsers.includes(key)} onChange={(event) => {
                updateConfig({ shown_browsers: event.target.checked
                  ? [...config.shown_browsers, key]
                  : config.shown_browsers.filter((item) => item !== key) })
              }} />
              <BrowserIcon icon={icon} />{label}
            </label>)}
            {targets.length === 0 && <p>No browsers detected.</p>}
            {staleShownBrowsers.map((key) => <label key={key}>
              <input type="checkbox" aria-label={`Show Not installed: ${key}`} checked onChange={() => {
                updateConfig({ shown_browsers: config.shown_browsers.filter((item) => item !== key) })
              }} />
              {`Not installed: ${key}`}
            </label>)}
          </div>
        </section>}
        {section === 'rules' && <section>
          <div className="section-heading"><h2>Rules</h2><div className="rule-actions"><button type="button" onClick={() => setDraft({ ...emptyDraft })}>Add rule</button><button type="button" aria-expanded={presetMenu} onClick={() => setPresetMenu(!presetMenu)}>Add preset</button></div></div>
          {presetMenu && <div className="preset-menu" role="menu" aria-label="Presets">{presets.map((preset) => <button type="button" role="menuitem" key={preset.id} onClick={() => addPreset(preset)}>{preset.name}</button>)}</div>}
          <p className="hint">Rules are checked from top to bottom. The first match wins.</p>
          {!caps.source_app_rules && <p className="hint">Source app rules are unavailable on Linux. Existing source app values are kept when you save.</p>}
          <ol aria-label="Rules" className="rules">
            {config.rules.map((rule, index) => <li key={index}>
              <div><h3>{rule.name}{rule.preset && <> <span className="preset-badge">{rule.preset}</span></>}</h3><p>{rule.match.join(', ')}</p></div>
              <div className="rule-actions">
                <button type="button" aria-label={`Move ${rule.name} up`} disabled={index === 0} onClick={() => moveRule(index, -1)}>↑</button>
                <button type="button" aria-label={`Move ${rule.name} down`} disabled={index === config.rules.length - 1} onClick={() => moveRule(index, 1)}>↓</button>
                <button type="button" aria-label={`Edit ${rule.name}`} onClick={() => editRule(index)}>Edit</button>
                <button type="button" aria-label={`Delete ${rule.name}`} onClick={() => updateRules(config.rules.filter((_, i) => i !== index))}>Delete</button>
              </div>
            </li>)}
          </ol>
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
            <div className="editor-actions"><button type="button" onClick={commitRule}>Save rule</button><button type="button" onClick={() => setDraft(null)}>Cancel</button></div>
          </div>}
        </section>}
        <footer><button type="button" className="primary-action" disabled={busy} onClick={() => void save()}>Save settings</button>{saved && <span role="status">Settings saved.</span>}</footer>
      </>}
      {!config && !error && <p>Loading settings…</p>}
    </main>
  )
}
