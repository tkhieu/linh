import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIcon, listPresets, saveConfig } from './api'
import type { Config } from './api'
import { config } from './test/fixtures'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

beforeEach(() => invoke.mockReset())

describe('preset and icon commands', () => {
  it('uses the frozen command names and icon argument', async () => {
    invoke.mockResolvedValueOnce([{ id: 'meet', name: 'Google Meet', patterns: ['meet.google.com'], app: null, app_installed: false }])
      .mockResolvedValueOnce('data:image/png;base64,aGVsbG8=')
    expect(await listPresets()).toHaveLength(1)
    expect(await getIcon('google-chrome')).toBe('data:image/png;base64,aGVsbG8=')
    expect(invoke).toHaveBeenNthCalledWith(1, 'list_presets')
    expect(invoke).toHaveBeenNthCalledWith(2, 'get_icon', { icon: 'google-chrome' })
  })

  it('passes AppTarget with fallback to save_config unchanged', async () => {
    invoke.mockResolvedValue(undefined)
    const next: Config = { ...config, rules: [{ name: 'Teams', preset: 'teams', match: ['teams.microsoft.com/meet/*'], open_in: { app: 'teams', fallback: config.primary } }] }
    await saveConfig(next)
    expect(invoke).toHaveBeenCalledWith('save_config', { config: next })
  })
})
