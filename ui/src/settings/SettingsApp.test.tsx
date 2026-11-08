import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { chrome, config, firefox, work } from '../test/fixtures'
import { targetKey } from '../targets'
import { SettingsApp } from './SettingsApp'

const api = vi.hoisted(() => ({
  listBrowsers: vi.fn(), getConfig: vi.fn(), saveConfig: vi.fn(),
  getPlatformCaps: vi.fn(), isDefaultBrowser: vi.fn(), setDefaultBrowser: vi.fn(),
}))
vi.mock('../api', () => api)

beforeEach(() => {
  vi.clearAllMocks()
  api.listBrowsers.mockResolvedValue([chrome, firefox])
  api.getConfig.mockResolvedValue(structuredClone(config))
  api.getPlatformCaps.mockResolvedValue({ source_app_rules: false })
  api.isDefaultBrowser.mockResolvedValue(true)
  api.saveConfig.mockResolvedValue(undefined)
})

describe('SettingsApp', () => {
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

  it('adds, edits, reorders, deletes rules, and saves target settings', async () => {
    render(<SettingsApp />)
    await screen.findByRole('heading', { name: 'Rules' })
    fireEvent.change(screen.getByLabelText('Primary browser'), { target: { value: targetKey(work) } })
    fireEvent.change(screen.getByLabelText('Prompt mode'), { target: { value: 'always' } })
    fireEvent.click(screen.getByLabelText('Show Google Chrome — Work'))
    fireEvent.click(screen.getByRole('button', { name: 'Add rule' }))
    fireEvent.change(screen.getByLabelText('Rule name'), { target: { value: 'Meet' } })
    fireEvent.change(screen.getByLabelText('URL patterns'), { target: { value: 'meet.google.com\n*.company.com' } })
    fireEvent.change(screen.getByLabelText('Open in'), { target: { value: targetKey(work) } })
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    expect(screen.getByText('Meet')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Add rule' }))
    fireEvent.change(screen.getByLabelText('Rule name'), { target: { value: 'Other' } })
    fireEvent.change(screen.getByLabelText('URL patterns'), { target: { value: 'example.com' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    fireEvent.click(screen.getByRole('button', { name: 'Move Other up' }))
    const ruleList = screen.getByRole('list', { name: 'Rules' })
    expect(within(ruleList).getAllByRole('heading', { level: 3 }).map((node) => node.textContent)).toEqual(['Other', 'Meet'])
    fireEvent.click(screen.getByRole('button', { name: 'Edit Meet' }))
    fireEvent.change(screen.getByLabelText('Rule name'), { target: { value: 'Meetings' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }))
    fireEvent.click(screen.getByRole('button', { name: 'Delete Other' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(api.saveConfig).toHaveBeenCalledWith({
      primary: work, prompt_mode: 'always', shown_browsers: [targetKey(work)],
      rules: [{ name: 'Meetings', match: ['meet.google.com', '*.company.com'], open_in: work }],
    }))
    await waitFor(() => expect(api.getConfig).toHaveBeenCalledTimes(2))
  })

  it('hides source-app editing on Linux and preserves existing values on save', async () => {
    api.getConfig.mockResolvedValue({
      ...config, rules: [{ name: 'Slack', match: ['example.com'], source_apps: ['Slack'], open_in: 'prompt' }],
    })
    render(<SettingsApp />)
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

  it('shows backend save errors', async () => {
    api.saveConfig.mockRejectedValue('Invalid rule')
    render(<SettingsApp />)
    await screen.findByRole('heading', { name: 'Rules' })
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Invalid rule')
  })
})
