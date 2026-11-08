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

beforeEach(() => {
  vi.clearAllMocks()
  api.listBrowsers.mockResolvedValue([chrome, firefox])
  api.getConfig.mockResolvedValue({ ...config, shown_browsers: [targetKey(work)] })
  api.getPromptUrl.mockResolvedValue('https://meet.google.com/abc')
  api.promptChoose.mockResolvedValue(undefined)
  api.promptCancel.mockResolvedValue(undefined)
})

describe('PromptApp', () => {
  it('shows the pending URL and chooses a shown target with 1 immediately', async () => {
    render(<PromptApp />)
    await screen.findByText('https://meet.google.com/abc')
    expect(screen.getByText('Google Chrome — Work')).toBeInTheDocument()
    expect(screen.queryByText('Firefox')).not.toBeInTheDocument()
    fireEvent.keyDown(document, { key: '1' })
    await waitFor(() => expect(api.promptChoose).toHaveBeenCalledWith(work))
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
