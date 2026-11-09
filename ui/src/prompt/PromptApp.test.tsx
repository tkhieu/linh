import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { chrome, config, firefox, work } from '../test/fixtures'
import { targetKey } from '../targets'
import { PromptApp } from './PromptApp'

const api = vi.hoisted(() => ({
  listBrowsers: vi.fn(), getConfig: vi.fn(), getPromptUrl: vi.fn(),
  promptChoose: vi.fn(), promptCancel: vi.fn(),
}))
vi.mock('../api', () => api)

const tauriEvent = vi.hoisted(() => ({ listen: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => tauriEvent)

/** Invokes the handler the component registered for `event`, as the
 * backend's real emit would. */
function emit(event: string, payload: unknown) {
  const handler = tauriEvent.listen.mock.calls.find(([name]) => name === event)?.[1] as
    | ((event: { payload: unknown }) => void)
    | undefined
  handler?.({ payload })
}

beforeEach(() => {
  vi.clearAllMocks()
  tauriEvent.listen.mockResolvedValue(() => {})
  api.listBrowsers.mockResolvedValue([chrome, firefox])
  api.getConfig.mockResolvedValue({ ...config, shown_browsers: [targetKey(work)] })
  api.getPromptUrl.mockResolvedValue('https://meet.google.com/abc')
  api.promptChoose.mockResolvedValue(undefined)
  api.promptCancel.mockResolvedValue(undefined)
})

describe('PromptApp', () => {
  it('shows the pending URL and chooses a shown target with 1 immediately', async () => {
    render(<PromptApp />)
    // Wait for the choice button itself (not just the URL text) so the
    // async load has fully landed before the keydown fires below —
    // otherwise the keydown can race ahead of `choices` being set.
    await screen.findByRole('button', { name: /Google Chrome — Work/ })
    expect(screen.getByText('https://meet.google.com/abc')).toBeInTheDocument()
    expect(screen.queryByText('Firefox')).not.toBeInTheDocument()
    fireEvent.keyDown(document, { key: '1' })
    await waitFor(() => expect(api.promptChoose).toHaveBeenCalledWith(work))
  })

  it('re-syncs the pending URL and resets selection when prompt-url-changed fires', async () => {
    api.getConfig.mockResolvedValue(config)
    render(<PromptApp />)
    await screen.findByRole('button', { name: /Firefox \(Private\)/ })

    // Move the selection away from the first choice before the event
    // arrives, so resetting to 0 is actually observable.
    fireEvent.keyDown(document, { key: 'ArrowDown' })
    fireEvent.keyDown(document, { key: 'ArrowDown' })
    await waitFor(() => expect(screen.getAllByRole('button')[2]).toHaveClass('selected'))

    expect(tauriEvent.listen).toHaveBeenCalledWith('prompt-url-changed', expect.any(Function))
    emit('prompt-url-changed', 'https://example.com/new')

    await screen.findByText('https://example.com/new')
    expect(screen.queryByText('https://meet.google.com/abc')).not.toBeInTheDocument()
    await waitFor(() => expect(screen.getAllByRole('button')[0]).toHaveClass('selected'))

    fireEvent.keyDown(document, { key: 'Enter' })
    await waitFor(() => expect(api.promptChoose).toHaveBeenCalledWith({ browser: 'chrome', private: false }))
  })

  it('does not let a stale initial getPromptUrl() overwrite a newer prompt-url-changed URL', async () => {
    let resolveInitialUrl: ((value: string | null) => void) | undefined
    api.getPromptUrl.mockImplementation(
      () => new Promise<string | null>((resolve) => { resolveInitialUrl = resolve }),
    )
    api.getConfig.mockResolvedValue(config)

    render(<PromptApp />)

    // The event fires while the initial getPromptUrl() call is still
    // pending.
    emit('prompt-url-changed', 'https://example.com/new')
    await screen.findByText('https://example.com/new')

    // The initial load now resolves with a different, stale URL.
    resolveInitialUrl?.('https://meet.google.com/abc')

    // Wait for that load's `.then` to actually run (it also sets the
    // target list, so a target button appearing proves it ran) before
    // asserting the stale URL never took over.
    await screen.findByRole('button', { name: /Firefox \(Private\)/ })
    expect(screen.getByText('https://example.com/new')).toBeInTheDocument()
    expect(screen.queryByText('https://meet.google.com/abc')).not.toBeInTheDocument()
  })

  it('ignores a stale target refresh that resolves after a newer one', async () => {
    let resolveInitialConfig: ((value: typeof config) => void) | undefined
    let resolveInitialBrowsers: ((value: (typeof firefox)[]) => void) | undefined
    api.getConfig
      .mockImplementationOnce(() => new Promise((resolve) => { resolveInitialConfig = resolve }))
      .mockResolvedValueOnce({ ...config, shown_browsers: [] })
    api.listBrowsers
      .mockImplementationOnce(() => new Promise((resolve) => { resolveInitialBrowsers = resolve }))
      .mockResolvedValueOnce([firefox])

    render(<PromptApp />)
    await screen.findByText('https://meet.google.com/abc')

    // The event triggers a second, newer refresh that resolves right away.
    // (Exact text match: "Firefox" must not also match "Firefox (Private)".)
    emit('prompt-url-changed', 'https://example.com/new')
    await screen.findByText('Firefox')
    expect(screen.queryByText('Google Chrome')).not.toBeInTheDocument()

    // The initial (older) refresh now resolves after the newer one already
    // landed; it must not overwrite the newer targets with its own.
    resolveInitialConfig?.({ ...config, shown_browsers: [] })
    resolveInitialBrowsers?.([chrome])

    await waitFor(() => expect(screen.getByText('Firefox')).toBeInTheDocument())
    expect(screen.queryByText('Google Chrome')).not.toBeInTheDocument()
  })

  it('uses arrows and Enter to choose, and mouse click chooses', async () => {
    api.getConfig.mockResolvedValue(config)
    render(<PromptApp />)
    await screen.findByRole('button', { name: /Firefox \(Private\)/ })
    fireEvent.keyDown(document, { key: 'ArrowDown' })
    fireEvent.keyDown(document, { key: 'Enter' })
    await waitFor(() => expect(api.promptChoose).toHaveBeenCalledWith({ browser: 'chrome', private: true }))
    fireEvent.click(screen.getByRole('button', { name: /Firefox \(Private\)/ }))
    await waitFor(() => expect(api.promptChoose).toHaveBeenCalledWith({ browser: 'firefox', private: true }))
  })

  it('supports number 9', async () => {
    api.listBrowsers.mockResolvedValue([
      ...Array.from({ length: 8 }, (_, index) => ({ ...firefox, id: `browser-${index}`, display_name: `Browser ${index}` })),
      firefox,
    ])
    api.getConfig.mockResolvedValue(config)
    render(<PromptApp />)
    await screen.findByRole('button', { name: /9 Browser 4/ })
    fireEvent.keyDown(document, { key: '9' })
    await waitFor(() => expect(api.promptChoose).toHaveBeenCalledWith({ browser: 'browser-4', private: false }))
  })

  it('cancels with Escape', async () => {
    render(<PromptApp />)
    await screen.findByText('https://meet.google.com/abc')
    fireEvent.keyDown(document, { key: 'Escape' })
    await waitFor(() => expect(api.promptCancel).toHaveBeenCalledOnce())
  })

  it('handles a missing pending URL without choosing', async () => {
    api.getPromptUrl.mockResolvedValue(null)
    render(<PromptApp />)
    expect(await screen.findByText('No link is waiting to be opened.')).toBeInTheDocument()
    fireEvent.keyDown(document, { key: '1' })
    expect(api.promptChoose).not.toHaveBeenCalled()
  })
})
