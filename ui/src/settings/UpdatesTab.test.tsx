import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { UpdateStatus } from '../api'
import { config } from '../test/fixtures'
import { SettingsApp } from './SettingsApp'

const api = vi.hoisted(() => ({
  getConfig: vi.fn(), listBrowsers: vi.fn(), getPlatformCaps: vi.fn(), isDefaultBrowser: vi.fn(),
  saveConfig: vi.fn(), getUpdateStatus: vi.fn(), checkForUpdatesNow: vi.fn(), installUpdate: vi.fn(),
  openReleasePage: vi.fn(),
}))
const events = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
  unlisten: vi.fn(),
}))
vi.mock('../api', () => ({
  ...api,
  UPDATE_STATUS_CHANGED_EVENT: 'updates://status-changed',
  UPDATE_DOWNLOAD_PROGRESS_EVENT: 'updates://download-progress',
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    events.handlers.set(name, handler)
    return events.unlisten
  }),
}))

const base: UpdateStatus = {
  current_version: '0.3.2', kind: 'rpm', capability: 'self_install', managed_by: null, last_checked: null,
  available: { version: '0.4.0', date: '2026-11-27T10:00:00Z', notes: '<script>alert(1)</script>\nFixed links' },
  last_error: null,
}

beforeEach(() => {
  vi.clearAllMocks()
  events.handlers.clear()
  api.getConfig.mockResolvedValue(structuredClone(config))
  api.listBrowsers.mockResolvedValue([])
  api.getPlatformCaps.mockResolvedValue({ source_app_rules: false, restrict_profile_private: false })
  api.isDefaultBrowser.mockResolvedValue(true)
  api.saveConfig.mockResolvedValue(undefined)
  api.getUpdateStatus.mockResolvedValue(base)
  api.checkForUpdatesNow.mockResolvedValue(base.available)
  api.installUpdate.mockResolvedValue(undefined)
  api.openReleasePage.mockResolvedValue(undefined)
})

async function openUpdates() {
  render(<SettingsApp />)
  await screen.findByRole('button', { name: 'Save settings' })
  fireEvent.click(screen.getByRole('button', { name: 'Updates' }))
  await screen.findByText('0.3.2')
}

describe('Updates tab', () => {
  it.each([
    ['rpm', 'self_install', 'Update now', 'Linux RPM package'],
    ['mac_app', 'download_only', 'Download', 'macOS app'],
    ['msix', 'none', null, 'Microsoft Store version'],
    ['dev', 'none', null, 'Development build'],
  ] as const)('shows %s capability actions', async (kind, capability, action, label) => {
    api.getUpdateStatus.mockResolvedValue({ ...base, kind, capability })
    await openUpdates()
    expect(screen.getByText(label)).toBeInTheDocument()
    expect(screen.getByText(capability === 'self_install' ? 'Install from this app' : capability === 'download_only' ? 'Download from the release page' : 'Check only')).toBeInTheDocument()
    expect(screen.getByText('Never')).toBeInTheDocument()
    expect(screen.getByText((_, element) => element?.classList.contains('updates-notes') ?? false))
      .toHaveTextContent('<script>alert(1)</script> Fixed links')
    expect(document.querySelector('script')).toBeNull()
    expect(screen.getByRole('button', { name: 'Check now' })).toBeEnabled()
    if (action) expect(screen.getByRole('button', { name: action })).toBeEnabled()
    else {
      expect(screen.queryByRole('button', { name: 'Update now' })).not.toBeInTheDocument()
      expect(screen.queryByRole('button', { name: 'Download' })).not.toBeInTheDocument()
    }
    if (kind === 'msix') expect(screen.getByText(/come from the Store/)).toBeInTheDocument()
    if (kind === 'dev') expect(screen.getByText(/Development build\. Install/)).toBeInTheDocument()
  })

  it('installs exactly the shown version and disables repeat actions while progress arrives', async () => {
    let finish!: () => void
    api.installUpdate.mockImplementation(() => new Promise<void>((resolve) => { finish = resolve }))
    await openUpdates()
    fireEvent.click(screen.getByRole('button', { name: 'Update now' }))
    expect(api.installUpdate).toHaveBeenCalledWith('0.4.0')
    expect(screen.getByRole('button', { name: 'Update now' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Check now' })).toBeDisabled()
    events.handlers.get('updates://download-progress')?.({ payload: { downloaded: 50, total: 100 } })
    expect(await screen.findByRole('status', { name: '' })).toHaveTextContent('Downloading: 50%')
    fireEvent.click(screen.getByRole('button', { name: 'Update now' }))
    expect(api.installUpdate).toHaveBeenCalledOnce()
    finish()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Update now' })).toBeEnabled())
  })

  it('opens the release page for download-only installs', async () => {
    api.getUpdateStatus.mockResolvedValue({ ...base, kind: 'mac_app', capability: 'download_only' })
    await openUpdates()
    fireEvent.click(screen.getByRole('button', { name: 'Download' }))
    await waitFor(() => expect(api.openReleasePage).toHaveBeenCalledOnce())
  })

  it('checks on demand and shows the backend error', async () => {
    api.getUpdateStatus.mockResolvedValue({ ...base, last_error: 'Password prompt cancelled' })
    await openUpdates()
    expect(screen.getByRole('alert')).toHaveTextContent('Password prompt cancelled')
    fireEvent.click(screen.getByRole('button', { name: 'Check now' }))
    await waitFor(() => expect(api.checkForUpdatesNow).toHaveBeenCalledOnce())
  })

  it('disables repeat checks until the current check finishes', async () => {
    let finish!: () => void
    api.checkForUpdatesNow.mockImplementation(() => new Promise<void>((resolve) => { finish = resolve }))
    api.getUpdateStatus.mockResolvedValue({ ...base, last_checked: Date.UTC(2026, 10, 27, 10) })
    await openUpdates()
    expect(screen.queryByText('Never')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Check now' }))
    expect(screen.getByRole('button', { name: 'Checking…' })).toBeDisabled()
    expect(api.checkForUpdatesNow).toHaveBeenCalledOnce()
    finish()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Check now' })).toBeEnabled())
  })

  it('rechecks a changed version and requires a new click', async () => {
    const newer = { ...base, available: { version: '0.4.1', notes: 'New fixes' }, last_error: null }
    api.getUpdateStatus.mockResolvedValueOnce(base).mockResolvedValue(newer)
    api.installUpdate.mockRejectedValue('a different version (0.4.1) is now available; re-check before installing')
    await openUpdates()
    fireEvent.click(screen.getByRole('button', { name: 'Update now' }))
    await waitFor(() => expect(api.checkForUpdatesNow).toHaveBeenCalledOnce())
    expect(await screen.findByText('Version 0.4.1 available')).toBeInTheDocument()
    expect(screen.getByRole('status')).toHaveTextContent('Review it, then choose Update now again')
    expect(api.installUpdate).toHaveBeenCalledTimes(1)
  })

  it('saves the automatic-check toggle through the settings config', async () => {
    await openUpdates()
    const toggle = screen.getByRole('checkbox', { name: 'Check for updates automatically' })
    expect(toggle).toBeChecked()
    fireEvent.click(toggle)
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith({ ...config, check_for_updates: false }))
  })

  it('refreshes on status events and removes listeners on unmount', async () => {
    const view = render(<SettingsApp />)
    await screen.findByRole('button', { name: 'Save settings' })
    fireEvent.click(screen.getByRole('button', { name: 'Updates' }))
    await screen.findByText('0.3.2')
    api.getUpdateStatus.mockResolvedValue({ ...base, available: { version: '0.4.1' } })
    events.handlers.get('updates://status-changed')?.({ payload: undefined })
    expect(await screen.findByText('Version 0.4.1 available')).toBeInTheDocument()
    view.unmount()
    expect(events.unlisten).toHaveBeenCalledTimes(2)
  })

  it.each([
    ['microsoft_store', 'the Microsoft Store'],
    ['mac_app_store', 'the Mac App Store'],
    ['flathub', 'Flathub'],
    ['copr', 'your package manager (Copr)'],
    ['ppa', 'your package manager (PPA)'],
  ] as const)('shows the managed-by message and hides checking controls for %s', async (managedBy, label) => {
    api.getUpdateStatus.mockResolvedValue({
      ...base, capability: 'none', managed_by: managedBy, available: null, last_error: null,
    })
    await openUpdates()
    expect(screen.getByText('0.3.2')).toBeInTheDocument()
    expect(screen.getByText(`Updates for this version come from ${label}.`)).toBeInTheDocument()
    expect(screen.getByText('Update method').closest('div')).toHaveTextContent(label)
    expect(screen.queryByRole('button', { name: 'Check now' })).not.toBeInTheDocument()
    expect(screen.queryByRole('checkbox', { name: 'Check for updates automatically' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Update now' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Download' })).not.toBeInTheDocument()
    expect(api.checkForUpdatesNow).not.toHaveBeenCalled()
  })
})
