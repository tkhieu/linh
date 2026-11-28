/// <reference types="node" />
import { readFileSync } from 'node:fs'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { chrome, config, firefox, work } from '../test/fixtures'
import { targetKey } from '../targets'
import { SettingsApp } from './SettingsApp'

const api = vi.hoisted(() => ({
  listBrowsers: vi.fn(), getConfig: vi.fn(), saveConfig: vi.fn(),
  getPlatformCaps: vi.fn(), isDefaultBrowser: vi.fn(), setDefaultBrowser: vi.fn(),
  listPresets: vi.fn(), getIcon: vi.fn(),
}))
vi.mock('../api', () => api)

beforeEach(() => {
  vi.clearAllMocks()
  api.listBrowsers.mockResolvedValue([chrome, firefox])
  api.getConfig.mockResolvedValue(structuredClone(config))
  api.getPlatformCaps.mockResolvedValue({ source_app_rules: false, restrict_profile_private: false })
  api.isDefaultBrowser.mockResolvedValue(true)
  api.saveConfig.mockResolvedValue(undefined)
  api.getIcon.mockResolvedValue(null)
  api.listPresets.mockResolvedValue([
    { id: 'meet', name: 'Google Meet', patterns: ['meet.google.com'], app: null, app_installed: false },
    { id: 'teams', name: 'Microsoft Teams', patterns: ['teams.microsoft.com/meet/*'], app: 'teams', app_installed: true },
    { id: 'slack', name: 'Slack', patterns: ['*.slack.com/archives/*'], app: null, app_installed: false },
    { id: 'zoom', name: 'Zoom', patterns: ['zoom.us/j/*'], app: 'zoom', app_installed: false },
  ])
})

function openSection(name: 'Browsers' | 'Rules') {
  fireEvent.click(screen.getByRole('button', { name }))
}

describe('SettingsApp', () => {
  it('keeps browser checkbox focus inside the scrollable list', async () => {
    const sheet = document.createElement('style')
    sheet.textContent = readFileSync('src/styles.css', 'utf8')
    document.head.append(sheet)
    try {
      render(<SettingsApp />)
      await screen.findByLabelText('Primary browser')
      openSection('Browsers')
      const checkbox = screen.getByRole('checkbox', { name: /^Show Google Chrome — Work$/ })
      const row = checkbox.closest('label')!
      expect(getComputedStyle(checkbox).position).toBe('absolute')
      expect(getComputedStyle(row).position).toBe('relative')

      fireEvent.click(checkbox)
      expect(checkbox).toBeInTheDocument()
      expect(checkbox).toBeChecked()
      expect(api.saveConfig).not.toHaveBeenCalled()
      expect(api.listBrowsers).toHaveBeenCalledOnce()
    } finally {
      sheet.remove()
    }
  })

  it('shows onboarding and rechecks default status after registration', async () => {
    api.isDefaultBrowser.mockResolvedValueOnce(false).mockResolvedValueOnce(true)
    api.setDefaultBrowser.mockResolvedValue(undefined)
    render(<SettingsApp />)
    fireEvent.click(await screen.findByRole('button', { name: 'Make Linh your default browser' }))
    await waitFor(() => expect(api.setDefaultBrowser).toHaveBeenCalledOnce())
    expect(await screen.findByText('Linh is your default browser.')).toBeInTheDocument()
  })

  it('shows registration errors', async () => {
    api.isDefaultBrowser.mockResolvedValue(false)
    api.setDefaultBrowser.mockRejectedValue('Registration failed')
    render(<SettingsApp />)
    fireEvent.click(await screen.findByRole('button', { name: 'Make Linh your default browser' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Registration failed')
  })

  it('explains the Windows Default Apps handoff without claiming success', async () => {
    const userAgent = vi.spyOn(window.navigator, 'userAgent', 'get').mockReturnValue('Windows NT 10.0')
    try {
      api.isDefaultBrowser.mockResolvedValue(false)
      api.setDefaultBrowser.mockResolvedValue(undefined)
      render(<SettingsApp />)
      fireEvent.click(await screen.findByRole('button', { name: 'Open Windows Default Apps settings' }))
      expect(await screen.findByRole('status')).toHaveTextContent('Choose Linh for HTTP and HTTPS')
      expect(api.setDefaultBrowser).toHaveBeenCalledOnce()
    } finally {
      userAgent.mockRestore()
    }
  })

  it('names the current platform in the source-app-rules hint', async () => {
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    expect(await screen.findByText(/Source app rules are unavailable on Linux\./)).toBeInTheDocument()
  })

  it('names macOS in the source-app-rules hint on a Mac', async () => {
    const userAgent = vi.spyOn(window.navigator, 'userAgent', 'get').mockReturnValue(
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)',
    )
    try {
      render(<SettingsApp />)
      await screen.findByLabelText('Primary browser')
      openSection('Rules')
      expect(await screen.findByText(/Source app rules are unavailable on macOS\./)).toBeInTheDocument()
    } finally {
      userAgent.mockRestore()
    }
  })

  it('hides profile and private options and explains why on a Mac App Store build', async () => {
    api.getPlatformCaps.mockResolvedValue({ source_app_rules: false, restrict_profile_private: true })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Browsers')
    expect(await screen.findByText(/aren't available in the Mac App Store version/)).toBeInTheDocument()
    expect(screen.getAllByText('—', { selector: '.col-private' }).length).toBeGreaterThan(0)
    openSection('Rules')
    fireEvent.click(screen.getByRole('button', { name: '+ New route' }))
    const options = within(screen.getByLabelText('Open in')).getAllByRole('option').map((option) => option.textContent)
    expect(options.some((label) => label?.includes('Private'))).toBe(false)
    expect(options.some((label) => label?.includes('—'))).toBe(false)
  })

  it('names Windows, not Linux, in the source-app-rules hint on Windows', async () => {
    const userAgent = vi.spyOn(window.navigator, 'userAgent', 'get').mockReturnValue('Windows NT 10.0')
    try {
      render(<SettingsApp />)
      await screen.findByLabelText('Primary browser')
      openSection('Rules')
      expect(await screen.findByText(/Source app rules are unavailable on Windows\./)).toBeInTheDocument()
    } finally {
      userAgent.mockRestore()
    }
  })

  it('adds, edits, reorders, deletes rules, and saves target settings', async () => {
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    fireEvent.change(screen.getByLabelText('Primary browser'), { target: { value: targetKey(work) } })
    fireEvent.change(screen.getByLabelText('Prompt mode'), { target: { value: 'always' } })
    openSection('Browsers')
    fireEvent.click(screen.getByLabelText('Show Google Chrome — Work'))
    openSection('Rules')
    fireEvent.click(screen.getByRole('button', { name: '+ New route' }))
    fireEvent.change(screen.getByLabelText('Rule name'), { target: { value: 'Meet' } })
    fireEvent.change(screen.getByLabelText('URL patterns'), { target: { value: 'meet.google.com\n*.company.com' } })
    fireEvent.change(screen.getByLabelText('Open in'), { target: { value: targetKey(work) } })
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    expect(screen.getByText('Meet')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '+ New route' }))
    fireEvent.change(screen.getByLabelText('Rule name'), { target: { value: 'Other' } })
    fireEvent.change(screen.getByLabelText('URL patterns'), { target: { value: 'example.com' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    fireEvent.click(screen.getByRole('button', { name: 'Move Other up' }))
    const ruleList = screen.getByRole('list', { name: 'Routes' })
    expect(within(ruleList).getAllByRole('heading', { level: 3 }).map((node) => node.textContent)).toEqual(['Other', 'Meet'])
    fireEvent.click(screen.getByRole('button', { name: 'Edit Meet' }))
    fireEvent.change(screen.getByLabelText('Rule name'), { target: { value: 'Meetings' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    fireEvent.click(screen.getByRole('button', { name: 'Delete Other' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith({
      primary: work, prompt_mode: 'always', shown_browsers: [targetKey(work)], check_for_updates: true,
      rules: [{ name: 'Meetings', match: ['meet.google.com', '*.company.com'], open_in: work }],
    }))
    await waitFor(() => expect(api.getConfig).toHaveBeenCalledTimes(2))
  })

  it('hides source-app editing on Linux and preserves existing values on save', async () => {
    api.getConfig.mockResolvedValue({
      ...config, rules: [{ name: 'Slack', match: ['example.com'], source_apps: ['Slack'], open_in: 'prompt' }],
    })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    await screen.findByText('Slack')
    expect(screen.queryByLabelText('Source apps')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Edit Slack' }))
    expect(screen.queryByLabelText('Source apps')).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Rule name'), { target: { value: 'Slack links' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith(expect.objectContaining({
      rules: [{ name: 'Slack links', match: ['example.com'], source_apps: ['Slack'], open_in: 'prompt' }],
    })))
  })

  it('preserves a stale shown-browser key when another target is selected and saved', async () => {
    api.getConfig.mockResolvedValue({ ...config, shown_browsers: ['retired-browser/Profile 9'] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Browsers')
    await screen.findByRole('heading', { name: 'Targets' })
    expect(screen.getByLabelText('Show Google Chrome — Work')).not.toBeChecked()
    fireEvent.click(screen.getByLabelText('Show Google Chrome — Work'))
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith(expect.objectContaining({
      shown_browsers: ['retired-browser/Profile 9', targetKey(work)],
    })))
  })

  it('lists a stale shown-browser key as removable and clears it on request', async () => {
    api.getConfig.mockResolvedValue({ ...config, shown_browsers: ['retired-browser/Profile 9'] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Browsers')
    await screen.findByRole('heading', { name: 'Targets' })
    const staleCheckbox = screen.getByLabelText('Show Not installed: retired-browser/Profile 9')
    expect(staleCheckbox).toBeChecked()
    fireEvent.click(staleCheckbox)
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith(expect.objectContaining({
      shown_browsers: [],
    })))
  })

  it('shows backend save errors', async () => {
    api.saveConfig.mockRejectedValue('Invalid rule')
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    await screen.findByRole('heading', { name: 'Routes' })
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Invalid rule')
  })

  it('navigates between General, Browsers, and Rules', async () => {
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    expect(screen.queryByRole('heading', { name: 'Routes' })).not.toBeInTheDocument()
    openSection('Browsers')
    expect(screen.getByRole('heading', { name: 'Targets' })).toBeInTheDocument()
    openSection('Rules')
    expect(screen.getByRole('heading', { name: 'Routes' })).toBeInTheDocument()
    expect(screen.queryByLabelText('Primary browser')).not.toBeInTheDocument()
  })

  it('adds a Meet preset with editable patterns and a browser profile', async () => {
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    fireEvent.click(screen.getByRole('button', { name: 'Presets' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Google Meet' }))
    expect(screen.getByLabelText('URL patterns')).toHaveValue('meet.google.com')
    fireEvent.change(screen.getByLabelText('URL patterns'), { target: { value: 'meet.google.com\nmeet.example.com' } })
    fireEvent.change(screen.getByLabelText('Open in'), { target: { value: targetKey(work) } })
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    expect(screen.getByText('meet')).toHaveClass('preset-badge')
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith(expect.objectContaining({ rules: [
      { name: 'Google Meet', preset: 'meet', match: ['meet.google.com', 'meet.example.com'], open_in: work },
    ] })))
  })

  it('adds Teams with an app fallback and reports installation state', async () => {
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    fireEvent.click(screen.getByRole('button', { name: 'Presets' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Microsoft Teams' }))
    expect(screen.getByLabelText('Open in')).toHaveValue('app:teams')
    expect(screen.getByText('App installed')).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Fallback browser'), { target: { value: targetKey(work) } })
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith(expect.objectContaining({ rules: [
      { name: 'Microsoft Teams', preset: 'teams', match: ['teams.microsoft.com/meet/*'], open_in: { app: 'teams', fallback: work } },
    ] })))
  })

  it('shows the fallback message for an absent app and round-trips AppTarget on edit', async () => {
    api.getConfig.mockResolvedValue({ ...config, rules: [
      { name: 'Zoom calls', preset: 'zoom', match: ['zoom.us/j/*'], open_in: { app: 'zoom', fallback: work } },
    ] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    fireEvent.click(screen.getByRole('button', { name: 'Edit Zoom calls' }))
    expect(screen.getByLabelText('Open in')).toHaveValue('app:zoom')
    expect(screen.getByLabelText('Fallback browser')).toHaveValue(targetKey(work))
    expect(await screen.findByText('App not found — links will open in the fallback browser')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith(expect.objectContaining({ rules: [
      { name: 'Zoom calls', preset: 'zoom', match: ['zoom.us/j/*'], open_in: { app: 'zoom', fallback: work } },
    ] })))
  })

  it('keeps a hand-written app rule without preset metadata', async () => {
    api.getConfig.mockResolvedValue({ ...config, rules: [
      { name: 'Custom Teams', match: ['teams.example.com/*'], open_in: { app: 'teams', fallback: work } },
    ] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    fireEvent.click(screen.getByRole('button', { name: 'Edit Custom Teams' }))
    expect(screen.getByLabelText('Open in')).toHaveValue('app:teams')
    expect(screen.getByLabelText('Fallback browser')).toHaveValue(targetKey(work))
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith(expect.objectContaining({ rules: [
      { name: 'Custom Teams', match: ['teams.example.com/*'], open_in: { app: 'teams', fallback: work } },
    ] })))
  })

  it('shows and preserves Prompt on a rule with Teams preset metadata', async () => {
    const rule = { name: 'Ask for Teams', preset: 'teams' as const, match: ['teams.example.com/*'], open_in: 'prompt' as const }
    api.getConfig.mockResolvedValue({ ...config, rules: [rule] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    fireEvent.click(screen.getByRole('button', { name: 'Edit Ask for Teams' }))
    const destination = screen.getByLabelText('Open in')
    expect(destination).toHaveValue('prompt')
    expect(within(destination).getByRole('option', { name: 'Prompt' })).toBeInTheDocument()
    expect(within(destination).getByRole('option', { name: 'Teams app' })).toBeInTheDocument()
    expect(within(destination).getByRole('option', { name: 'Zoom app' })).toBeInTheDocument()
    expect(within(destination).getByRole('option', { name: 'Google Chrome — Work' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith(expect.objectContaining({ rules: [rule] })))
  })

  it('shows and preserves Zoom with fallback on a rule with Teams preset metadata', async () => {
    const rule = { name: 'Zoom via Teams preset', preset: 'teams' as const, match: ['zoom.example.com/*'], open_in: { app: 'zoom' as const, fallback: work } }
    api.getConfig.mockResolvedValue({ ...config, rules: [rule] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    fireEvent.click(screen.getByRole('button', { name: 'Edit Zoom via Teams preset' }))
    expect(screen.getByLabelText('Open in')).toHaveValue('app:zoom')
    expect(screen.getByLabelText('Fallback browser')).toHaveValue(targetKey(work))
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith(expect.objectContaining({ rules: [rule] })))
  })

  it('refreshes app installation status each time Rules is opened', async () => {
    const teams = { id: 'teams', name: 'Microsoft Teams', patterns: ['teams.microsoft.com/meet/*'], app: 'teams' }
    api.listPresets.mockResolvedValueOnce([{ ...teams, app_installed: true }])
      .mockResolvedValueOnce([{ ...teams, app_installed: false }])
    api.getConfig.mockResolvedValue({ ...config, rules: [
      { name: 'Teams links', preset: 'teams', match: ['teams.example.com/*'], open_in: { app: 'teams' } },
    ] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    fireEvent.click(screen.getByRole('button', { name: 'Edit Teams links' }))
    expect(await screen.findByText('App installed')).toBeInTheDocument()
    openSection('Browsers')
    openSection('Rules')
    expect(await screen.findByText('App not found — links will open in the fallback browser')).toBeInTheDocument()
    expect(api.listPresets).toHaveBeenCalledTimes(2)
  })

  it('renders browser icons and a placeholder for an unavailable icon', async () => {
    api.listBrowsers.mockResolvedValue([{ ...chrome, icon: 'chrome-icon' }, { ...firefox, icon: 'missing' }])
    api.getIcon.mockImplementation((icon: string) => Promise.resolve(icon === 'chrome-icon' ? 'data:image/png;base64,aGVsbG8=' : null))
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Browsers')
    await waitFor(() => expect(document.querySelectorAll('.checklist img.browser-icon')).toHaveLength(4))
    expect(screen.getAllByText('◎').length).toBeGreaterThan(0)
    expect(api.getIcon).toHaveBeenCalledTimes(2)
  })

  it('rescan refreshes detected browsers without discarding an unsaved edit', async () => {
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    fireEvent.change(screen.getByLabelText('Primary browser'), { target: { value: targetKey(work) } })
    openSection('Browsers')
    const listBrowsersCallsBeforeRescan = api.listBrowsers.mock.calls.length
    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))
    await waitFor(() => expect(api.listBrowsers.mock.calls.length).toBeGreaterThan(listBrowsersCallsBeforeRescan))
    // getConfig must not be called again by rescan — only the detected
    // browsers/caps/default-status, never the edited config draft.
    expect(api.getConfig).toHaveBeenCalledTimes(1)
    fireEvent.click(screen.getByRole('button', { name: 'General' }))
    expect(screen.getByLabelText('Primary browser')).toHaveValue(targetKey(work))
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith(expect.objectContaining({ primary: work })))
  })

  it('shows routes before the unmatched picker in Current behaviour under Always', async () => {
    api.getConfig.mockResolvedValue({ ...config, prompt_mode: 'always' })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    expect(screen.getByText('Check routes')).toBeInTheDocument()
    expect(screen.getByText('No match')).toBeInTheDocument()
    expect(screen.getByText('Picker')).toBeInTheDocument()
    expect(screen.getByText('Always: unmatched links open the picker. Never: unmatched links open Firefox.')).toBeInTheDocument()
  })

  it('test a link follows a matching route under Always, then prompts for an unmatched link', async () => {
    api.getConfig.mockResolvedValue({
      ...config, prompt_mode: 'always',
      rules: [{ name: 'Meet', match: ['meet.google.com'], open_in: work }],
    })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    fireEvent.change(screen.getByLabelText('Test a link'), { target: { value: 'https://meet.google.com' } })
    const sidePanel = screen.getByText('Test a link').closest('aside')!
    expect(within(sidePanel).getByText('Meet')).toBeInTheDocument()
    expect(within(sidePanel).getByText('Google Chrome — Work')).toBeInTheDocument()
    expect(within(sidePanel).getByText('via route 01 · meet.google.com')).toBeInTheDocument()
    fireEvent.change(within(sidePanel).getByLabelText('Test a link'), { target: { value: 'https://unmatched.example' } })
    expect(within(sidePanel).getByText('Picker')).toBeInTheDocument()
    expect(within(sidePanel).getByText('no match · prompt mode: always')).toBeInTheDocument()
  })

  it('shows the stored scheme verbatim and previews its Github route to Chrome', async () => {
    const macChrome = { ...chrome, id: 'com.google.chrome' }
    const macTarget = { browser: macChrome.id, private: false }
    api.listBrowsers.mockResolvedValue([macChrome])
    api.getConfig.mockResolvedValue({
      primary: macTarget, prompt_mode: 'always', shown_browsers: [],
      rules: [{ name: 'Github', match: ['https://github.com/*'], open_in: macTarget }],
    })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')

    const route = screen.getByText('Github').closest('li')!
    expect(within(route).getByText('https://github.com/*')).toBeInTheDocument()
    const sidePanel = screen.getByText('Test a link').closest('aside')!
    fireEvent.change(within(sidePanel).getByLabelText('Test a link'), {
      target: { value: 'https://github.com/tkhieu/linh/pull/29' },
    })
    expect(within(sidePanel).getByText('Google Chrome')).toBeInTheDocument()
    expect(within(sidePanel).getByText('via route 01 · github.com')).toBeInTheDocument()

    fireEvent.click(within(route).getByRole('button', { name: 'Edit Github' }))
    expect(screen.getByLabelText('URL patterns')).toHaveValue('https://github.com/*')
  })

  it('shows "not a routable link" for a non-http(s) link, and never prefixes https:// onto a bare host', async () => {
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    const input = screen.getByLabelText('Test a link')
    fireEvent.change(input, { target: { value: 'mailto:foo@bar.com' } })
    expect(await screen.findByText('Not a routable link — only http:// and https:// links are routed.')).toBeInTheDocument()
    fireEvent.change(input, { target: { value: 'github.com/owner/repo' } })
    expect(await screen.findByText('Not a routable link — only http:// and https:// links are routed.')).toBeInTheDocument()
  })

  it('falls back consistently in the Routes table and Test a link once the Zoom app is confirmed not installed', async () => {
    api.getConfig.mockResolvedValue({ ...config, rules: [{ name: 'Zoom calls', match: ['zoom.us/j/*'], open_in: { app: 'zoom' } }] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    const routesList = screen.getByRole('list', { name: 'Routes' })
    const zoomRow = screen.getByText('Zoom calls').closest('li')!
    // listPresets() is still in flight: shows the app itself with a
    // neutral note, never a guessed fallback.
    expect(within(zoomRow).getByText('checking…')).toBeInTheDocument()
    expect(within(zoomRow).queryByText('Firefox')).not.toBeInTheDocument()
    await waitFor(() => expect(within(zoomRow).getByText('Firefox')).toBeInTheDocument())
    expect(routesList).toContainElement(zoomRow)

    const sidePanel = screen.getByText('Test a link').closest('aside')!
    fireEvent.change(within(sidePanel).getByLabelText('Test a link'), { target: { value: 'https://zoom.us/j/123456789' } })
    expect(await within(sidePanel).findByText('Zoom app not installed')).toBeInTheDocument()
    expect(within(sidePanel).getByText('Firefox')).toBeInTheDocument()
  })

  it('shows the app itself with a neutral note (never a guessed fallback) while availability is unknown, in both panels', async () => {
    let resolvePresets: ((value: Awaited<ReturnType<typeof api.listPresets>>) => void) | undefined
    api.listPresets.mockImplementation(() => new Promise((resolve) => { resolvePresets = resolve }))
    api.getConfig.mockResolvedValue({ ...config, rules: [{ name: 'Teams calls', match: ['teams.example.com/*'], open_in: { app: 'teams' } }] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    const zoomRow = screen.getByText('Teams calls').closest('li')!
    expect(within(zoomRow).getByText('Teams')).toBeInTheDocument()
    expect(within(zoomRow).getByText('checking…')).toBeInTheDocument()

    const sidePanel = screen.getByText('Test a link').closest('aside')!
    fireEvent.change(within(sidePanel).getByLabelText('Test a link'), { target: { value: 'https://teams.example.com/meet' } })
    expect(await within(sidePanel).findByText('checking…')).toBeInTheDocument()
    expect(within(sidePanel).getByText('Teams')).toBeInTheDocument()

    resolvePresets?.([{ id: 'teams', name: 'Microsoft Teams', patterns: ['teams.example.com/*'], app: 'teams', app_installed: true }])
    await waitFor(() => expect(within(zoomRow).queryByText('checking…')).not.toBeInTheDocument())
    expect(within(zoomRow).getByText('Teams')).toBeInTheDocument()
  })

  it('shows "app availability unknown" (not a guessed fallback) when listPresets() fails', async () => {
    api.listPresets.mockRejectedValue('network error')
    api.getConfig.mockResolvedValue({ ...config, rules: [{ name: 'Zoom calls', match: ['zoom.us/j/*'], open_in: { app: 'zoom' } }] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    const zoomRow = screen.getByText('Zoom calls').closest('li')!
    await waitFor(() => expect(within(zoomRow).getByText('app availability unknown')).toBeInTheDocument())
    expect(within(zoomRow).getByText('Zoom')).toBeInTheDocument()
    expect(within(zoomRow).queryByText('Firefox')).not.toBeInTheDocument()

    const sidePanel = screen.getByText('Test a link').closest('aside')!
    fireEvent.change(within(sidePanel).getByLabelText('Test a link'), { target: { value: 'https://zoom.us/j/123456789' } })
    expect(await within(sidePanel).findByText('app availability unknown')).toBeInTheDocument()
    expect(within(sidePanel).getByText('Zoom')).toBeInTheDocument()
  })

  it('still falls back for a non-convertible Zoom link while availability is unknown — that part is a url fact', async () => {
    api.listPresets.mockImplementation(() => new Promise(() => { /* never resolves */ }))
    api.getConfig.mockResolvedValue({ ...config, rules: [{ name: 'Zoom calls', match: ['zoom.us/*'], open_in: { app: 'zoom' } }] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    const sidePanel = screen.getByText('Test a link').closest('aside')!
    fireEvent.change(within(sidePanel).getByLabelText('Test a link'), { target: { value: 'https://zoom.us/pricing' } })
    expect(await within(sidePanel).findByText('link not a Zoom meeting')).toBeInTheDocument()
    expect(within(sidePanel).getByText('Firefox')).toBeInTheDocument()
  })

  it('reports "link not a Zoom meeting" (not app-not-installed) for a non-convertible Zoom url, even when the app is installed', async () => {
    api.listPresets.mockResolvedValue([
      { id: 'zoom', name: 'Zoom', patterns: ['zoom.us/j/*'], app: 'zoom', app_installed: true },
    ])
    api.getConfig.mockResolvedValue({ ...config, rules: [{ name: 'Zoom calls', match: ['zoom.us/*'], open_in: { app: 'zoom' } }] })
    render(<SettingsApp />)
    await screen.findByLabelText('Primary browser')
    openSection('Rules')
    const sidePanel = screen.getByText('Test a link').closest('aside')!
    fireEvent.change(within(sidePanel).getByLabelText('Test a link'), { target: { value: 'https://zoom.us/pricing' } })
    expect(await within(sidePanel).findByText('link not a Zoom meeting')).toBeInTheDocument()
  })
})
