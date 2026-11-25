import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { currentMonitor, getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'
import { getConfig, getPromptUrl, listBrowsers, promptCancel, promptChoose } from '../api'
import type { BrowserTarget } from '../targets'
import { deriveTargets, visibleTargets } from '../targets'
import { BrowserIcon } from '../BrowserIcon'
import { promptSize } from './size'

function urlHost(url: string): string {
  try { return new URL(url).host } catch { return url }
}

export function PromptApp() {
  const [url, setUrl] = useState<string | null | undefined>(undefined)
  const [targets, setTargets] = useState<BrowserTarget[]>([])
  const [targetsLoaded, setTargetsLoaded] = useState(false)
  const [selected, setSelected] = useState(0)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const mainRef = useRef<HTMLElement>(null)
  const listRef = useRef<HTMLOListElement>(null)
  const choices = targets
  // Set once a prompt-url-changed event has applied, so the initial
  // getPromptUrl() result — which can resolve *after* a newer event,
  // since it races the event listener's own async setup — never
  // overwrites it with a stale URL.
  const urlSetByEventRef = useRef(false)

  useLayoutEffect(() => {
    if (url === undefined || !targetsLoaded) return
    const main = mainRef.current
    const list = listRef.current
    if (!main) return
    let active = true
    // Restore the natural list height before measuring a refreshed prompt.
    if (list) {
      list.style.maxHeight = ''
      list.style.overflowY = ''
    }
    const contentHeight = main.scrollHeight
    const listHeight = list?.scrollHeight ?? 0
    void (async () => {
      try {
        const monitor = await currentMonitor()
        if (!active) return
        const workAreaHeight = monitor ? monitor.workArea.size.height / monitor.scaleFactor : window.screen.availHeight
        const size = promptSize(contentHeight, workAreaHeight, listHeight)
        if (list && size.maxListHeight !== null) {
          list.style.maxHeight = `${size.maxListHeight}px`
          list.style.overflowY = 'auto'
        }
        const promptWindow = getCurrentWindow()
        await promptWindow.setSize(new LogicalSize(size.width, size.height))
        if (active) await promptWindow.center()
      } catch (reason) {
        console.error('Failed to size prompt window', reason)
      }
    })()
    return () => { active = false }
  }, [url, targets, targetsLoaded, error])

  useEffect(() => {
    let mounted = true
    let unlisten: (() => void) | undefined
    mainRef.current?.focus()

    // Refreshes can overlap (an event can fire again before the initial
    // load's fetch resolves). Each call claims the next sequence number
    // and only applies its result if it's still the most recently
    // started one, so an older, slower fetch can never clobber a newer
    // one's targets.
    let latestTargetsRequest = 0

    // Re-fetches the browser/config-derived target list so a picker
    // already on screen reflects the latest shown_browsers and
    // installed browsers, not just what was true when it first loaded.
    async function refreshTargets() {
      const requestId = ++latestTargetsRequest
      try {
        const [config, browsers] = await Promise.all([getConfig(), listBrowsers()])
        if (!mounted || requestId !== latestTargetsRequest) return
        setTargets(visibleTargets(deriveTargets(browsers), config.shown_browsers))
        setTargetsLoaded(true)
      } catch (reason) {
        if (mounted && requestId === latestTargetsRequest) {
          setError(String(reason))
          setTargetsLoaded(true)
        }
      }
    }

    async function run() {
      // Awaited before the initial fetch starts, so an event firing
      // while that fetch is in flight is never missed — listen()
      // itself resolves asynchronously, so starting the fetch first
      // would leave a real gap.
      unlisten = await listen<string>('prompt-url-changed', (event) => {
        if (!mounted) return
        urlSetByEventRef.current = true
        setUrl(event.payload)
        setSelected(0)
        setError('')
        mainRef.current?.focus()
        void refreshTargets()
      })
      if (!mounted) {
        unlisten()
        return
      }

      const loadUrl = (async () => {
        try {
          const pending = await getPromptUrl()
          if (!mounted) return
          if (!urlSetByEventRef.current) setUrl(pending)
          mainRef.current?.focus()
        } catch (reason) {
          if (mounted) setError(String(reason))
        }
      })()
      await Promise.all([loadUrl, refreshTargets()])
    }
    void run()

    return () => {
      mounted = false
      unlisten?.()
    }
  }, [])

  async function choose(index: number) {
    if (busy || !url || !choices[index]) return
    setBusy(true)
    setError('')
    try {
      await promptChoose(choices[index].target)
    } catch (reason) {
      setError(String(reason))
    } finally {
      setBusy(false)
    }
  }

  async function cancel() {
    if (busy) return
    setBusy(true)
    setError('')
    try {
      await promptCancel()
    } catch (reason) {
      setError(String(reason))
    } finally {
      setBusy(false)
    }
  }

  // Updated synchronously during render (not inside a useEffect), so it
  // is guaranteed current by the time this render commits — no
  // scheduling gap for the stable handler below to read a stale value
  // through. choose/cancel close over url/choices/selected/busy fresh
  // each render too, so refreshing them the same way keeps the handler
  // decoupled from React's effect-scheduling timing entirely.
  const latestRef = useRef({ url, choices, selected, choose, cancel })
  latestRef.current = { url, choices, selected, choose, cancel }

  // Attached exactly once, for the component's whole lifetime — never
  // detached and re-attached on a later render, which previously left a
  // narrow window where a keydown fired between unmounting the old
  // listener and mounting the new one would be silently lost.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const { url, choices, selected, choose, cancel } = latestRef.current
      if (event.key === 'Escape') {
        event.preventDefault()
        void cancel()
      } else if (url && choices.length) {
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
          event.preventDefault()
          setSelected((current) => (current + (event.key === 'ArrowDown' ? 1 : -1) + choices.length) % choices.length)
        } else if (event.key === 'Enter') {
          event.preventDefault()
          void choose(selected)
        } else if (/^[1-9]$/.test(event.key)) {
          event.preventDefault()
          void choose(Number(event.key) - 1)
        }
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [])

  return (
    <main className="prompt" ref={mainRef} tabIndex={-1}>
      <h1 className="sr-only">Choose a browser</h1>
      {url === undefined && !error && <p className="prompt-loading">Loading link…</p>}
      {url === null && <p className="prompt-loading">No link is waiting to be opened.</p>}
      {url && <>
        <div className="prompt-incoming">
          <p className="eyebrow">Incoming link</p>
          <p className="prompt-host">{urlHost(url)}</p>
          <p className="prompt-url" title={url}><span>{url}</span></p>
        </div>
        {choices.length === 0
          ? <p className="prompt-empty">No browser targets are available.</p>
          : <div className="prompt-targets">
            <div className="prompt-head-row"><span className="eyebrow">Depart via</span><span className="eyebrow">Key</span></div>
            <ol className="prompt-choices" ref={listRef}>
              {choices.map(({ key, label, icon }, index) => (
                <li key={key}>
                  <button
                    type="button"
                    className={index === selected ? 'selected' : ''}
                    aria-current={index === selected ? 'true' : undefined}
                    disabled={busy}
                    onMouseEnter={() => setSelected(index)}
                    onClick={() => void choose(index)}
                  >
                    {index < 9 && <kbd>{index + 1}</kbd>}<BrowserIcon icon={icon} /><span className="prompt-choice-label">{label}</span>
                  </button>
                </li>
              ))}
            </ol>
          </div>}
      </>}
      {error && <p role="alert" className="error">{error}</p>}
      <p className="hint prompt-shortcuts"><span>1–9 open</span><span>↑↓ select</span><span>↵ open</span><span>esc cancel</span></p>
    </main>
  )
}
