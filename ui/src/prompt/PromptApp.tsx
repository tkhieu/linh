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

  useEffect(() => {
    let mounted = true
    mainRef.current?.focus()
    Promise.all([getPromptUrl(), getConfig(), listBrowsers()]).then(([pending, config, browsers]) => {
      if (!mounted) return
      setUrl(pending)
      setTargets(visibleTargets(deriveTargets(browsers), config.shown_browsers))
      mainRef.current?.focus()
    }).catch((reason: unknown) => {
      if (mounted) setError(String(reason))
    })

    // A later launch can replace the pending URL while the prompt is
    // already open (the backend's prompt state machine keeps "latest
    // wins"); re-sync the displayed URL and reset the selection rather
    // than relying on a fresh getPromptUrl() poll.
    const unlisten = listen<string>('prompt-url-changed', (event) => {
      if (!mounted) return
      setUrl(event.payload)
      setSelected(0)
      setError('')
      mainRef.current?.focus()
    })

    return () => {
      mounted = false
      void unlisten.then((stop) => stop())
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

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
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
  })

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
