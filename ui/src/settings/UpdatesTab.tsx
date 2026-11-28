import { listen } from '@tauri-apps/api/event'
import { useEffect, useRef, useState } from 'react'
import {
  checkForUpdatesNow, getUpdateStatus, installUpdate, openReleasePage,
  UPDATE_DOWNLOAD_PROGRESS_EVENT, UPDATE_STATUS_CHANGED_EVENT,
} from '../api'
import type { ManagedBy, UpdateDownloadProgress, UpdateKind, UpdateStatus } from '../api'

const kindLabels: Record<UpdateKind, string> = {
  rpm: 'Linux RPM package',
  deb: 'Linux DEB package',
  nsis: 'Windows installer',
  msix: 'Microsoft Store version',
  mac_app: 'macOS app',
  dev: 'Development build',
}

const capabilityLabels: Record<UpdateStatus['capability'], string> = {
  self_install: 'Install from this app',
  download_only: 'Download from the release page',
  none: 'Check only',
}

/** Where the "Updates for this version come from …" message points, per `managed_by`. */
const managedByLabels: Record<ManagedBy, string> = {
  microsoft_store: 'the Microsoft Store',
  mac_app_store: 'the Mac App Store',
  flathub: 'Flathub',
  copr: 'your package manager (Copr)',
  ppa: 'your package manager (PPA)',
}

function message(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason)
}

function isVersionMismatch(reason: string): boolean {
  return reason.includes('re-check before installing')
}

function dateText(value: string | number): string {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString()
}

interface UpdatesTabProps {
  checkAutomatically: boolean
  onCheckAutomaticallyChange: (value: boolean) => void
}

export function UpdatesTab({ checkAutomatically, onCheckAutomaticallyChange }: UpdatesTabProps) {
  const [status, setStatus] = useState<UpdateStatus | null>(null)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState<'checking' | 'installing' | 'opening' | null>(null)
  const [progress, setProgress] = useState<UpdateDownloadProgress | null>(null)
  const busyRef = useRef(false)
  const requestId = useRef(0)

  async function refresh() {
    const id = ++requestId.current
    try {
      const next = await getUpdateStatus()
      if (id === requestId.current) setStatus(next)
    } catch (reason) {
      if (id === requestId.current) setError(message(reason))
    }
  }

  useEffect(() => {
    let active = true
    const requests = requestId
    let unlistenStatus: (() => void) | undefined
    let unlistenProgress: (() => void) | undefined
    void (async () => {
      try {
        unlistenStatus = await listen(UPDATE_STATUS_CHANGED_EVENT, () => {
          if (active) void refresh()
        })
        if (!active) { unlistenStatus(); return }
        unlistenProgress = await listen<UpdateDownloadProgress>(UPDATE_DOWNLOAD_PROGRESS_EVENT, (event) => {
          if (active && busyRef.current) setProgress(event.payload)
        })
        if (!active) { unlistenProgress(); return }
      } catch (reason) {
        if (active) setError(message(reason))
      }
      if (active) void refresh()
    })()
    return () => {
      active = false
      requests.current++
      unlistenStatus?.()
      unlistenProgress?.()
    }
  }, [])

  async function check() {
    if (busyRef.current) return
    busyRef.current = true
    setBusy('checking')
    setError('')
    setNotice('')
    try {
      await checkForUpdatesNow()
    } catch (reason) {
      setError(message(reason))
    } finally {
      await refresh()
      busyRef.current = false
      setBusy(null)
    }
  }

  async function install(version: string) {
    if (busyRef.current) return
    busyRef.current = true
    setBusy('installing')
    setProgress(null)
    setError('')
    setNotice('')
    try {
      await installUpdate(version)
    } catch (reason) {
      const detail = message(reason)
      if (isVersionMismatch(detail)) {
        try {
          await checkForUpdatesNow()
          await refresh()
          setNotice('The available version changed. Review it, then choose Update now again.')
        } catch (checkReason) {
          setError(message(checkReason))
        }
      } else {
        setError(detail)
      }
    } finally {
      busyRef.current = false
      setBusy(null)
      setProgress(null)
    }
  }

  async function download() {
    if (busyRef.current) return
    busyRef.current = true
    setBusy('opening')
    setError('')
    try {
      await openReleasePage()
    } catch (reason) {
      setError(message(reason))
    } finally {
      busyRef.current = false
      setBusy(null)
    }
  }

  const managedBy = status?.managed_by
  const available = status?.available
  const canInstall = status?.capability === 'self_install' && available
  const canDownload = status?.capability === 'download_only' && available
  const progressText = progress
    ? progress.total && progress.total > 0
      ? `Downloading: ${Math.min(100, Math.floor(progress.downloaded / progress.total * 100))}%`
      : `Downloading: ${Math.round(progress.downloaded / 1024)} KB`
    : 'Preparing update…'

  return <>
    <div className="settings-heading">
      <div><h2>Updates</h2><p>Choose when to install a new version.</p></div>
      {!managedBy && <div className="rule-actions"><button type="button" className="secondary" disabled={busy !== null} onClick={() => void check()}>{busy === 'checking' ? 'Checking…' : 'Check now'}</button></div>}
    </div>
    <dl className="updates-details">
      <div><dt>Current version</dt><dd>{status?.current_version ?? 'Loading…'}</dd></div>
      <div><dt>Install type</dt><dd>{status ? kindLabels[status.kind] : 'Loading…'}</dd></div>
      <div><dt>Update method</dt><dd>{status ? (managedBy ? managedByLabels[managedBy] : capabilityLabels[status.capability]) : 'Loading…'}</dd></div>
      <div><dt>Last checked</dt><dd>{status?.last_checked != null ? dateText(status.last_checked) : 'Never'}</dd></div>
    </dl>
    {!managedBy && <label className="updates-toggle">
      <input type="checkbox" checked={checkAutomatically} onChange={(event) => onCheckAutomaticallyChange(event.target.checked)} />
      Check for updates automatically
    </label>}
    {!managedBy && <p className="hint">This checks for new versions. Installing always needs your choice.</p>}
    {managedBy
      ? <p className="hint">Updates for this version come from {managedByLabels[managedBy]}.</p>
      : status?.capability === 'none' && <p className="hint">{status.kind === 'msix'
        ? 'Updates for the Microsoft Store version come from the Store.'
        : 'Development build. Install a packaged release to update Linh.'}</p>}
    {!managedBy && status?.capability === 'download_only' && <p className="hint">This copy can download releases but cannot install them itself.</p>}
    {!managedBy && status?.capability === 'self_install' && (status.kind === 'rpm' || status.kind === 'deb') &&
      <p className="hint">Installing on Linux asks for your password.</p>}
    {available && <section className="updates-available" aria-label="Available update">
      <h3>Version {available.version} available</h3>
      {available.date && <p>Released {dateText(available.date)}</p>}
      {available.notes && <><h4>Release notes</h4><p className="updates-notes">{available.notes}</p></>}
      {canInstall && <button type="button" className="primary-action" disabled={busy !== null} onClick={() => void install(available.version)}>Update now</button>}
      {canDownload && <button type="button" className="primary-action" disabled={busy !== null} onClick={() => void download()}>Download</button>}
    </section>}
    {status && !available && <p className="hint">No update available.</p>}
    {busy === 'installing' && <p role="status">{progressText}</p>}
    {notice && <p role="status">{notice}</p>}
    {status?.last_error && <p role="alert" className="error">Last update error: {status.last_error}</p>}
    {error && error !== status?.last_error && <p role="alert" className="error">{error}</p>}
  </>
}
