import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { getConfig, getPromptUrl, listBrowsers, promptCancel, promptChoose } from '../api'
import type { BrowserTarget } from '../targets'
import { deriveTargets, visibleTargets } from '../targets'

export function PromptApp() {
  const [url, setUrl] = useState<string | null | undefined>(undefined)
  const [targets, setTargets] = useState<BrowserTarget[]>([])
  const [selected, setSelected] = useState(0)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const mainRef = useRef<HTMLElement>(null)
  const choices = targets
  // Set once a prompt-url-changed event has applied, so the initial
  // getPromptUrl() result — which can resolve *after* a newer event,
  // since it races the event listener's own async setup — never
  // overwrites it with a stale URL.
  const urlSetByEventRef = useRef(false)

  useEffect(() => {
    let mounted = true
    let unlisten: (() => void) | undefined
    mainRef.current?.focus()

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
      })
      if (!mounted) {
        unlisten()
        return
      }

      try {
        const [pending, config, browsers] = await Promise.all([getPromptUrl(), getConfig(), listBrowsers()])
        if (!mounted) return
        if (!urlSetByEventRef.current) setUrl(pending)
        setTargets(visibleTargets(deriveTargets(browsers), config.shown_browsers))
        mainRef.current?.focus()
      } catch (reason) {
        if (mounted) setError(String(reason))
      }
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
      <h1>Choose a browser</h1>
      {url === undefined && !error && <p>Loading link…</p>}
      {url === null && <p>No link is waiting to be opened.</p>}
      {url && <>
        <p className="pending-url" title={url}>{url}</p>
        {choices.length === 0 && <p>No browser targets are available.</p>}
        <ol className="prompt-choices">
          {choices.map(({ key, label }, index) => (
            <li key={key}>
              <button
                type="button"
                className={index === selected ? 'selected' : ''}
                aria-current={index === selected ? 'true' : undefined}
                disabled={busy}
                onMouseEnter={() => setSelected(index)}
                onClick={() => void choose(index)}
              >
                {index < 9 && <kbd>{index + 1}</kbd>}<span>{label}</span>
              </button>
            </li>
          ))}
        </ol>
      </>}
      {error && <p role="alert" className="error">{error}</p>}
      <p className="hint">↑ ↓ to select · Enter to open · Esc to cancel</p>
    </main>
  )
}
