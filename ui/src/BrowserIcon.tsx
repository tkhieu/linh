import { useEffect, useState } from 'react'
import { getIcon } from './api'

const cache = new Map<string, Promise<string | null>>()

function loadIcon(icon: string): Promise<string | null> {
  let pending = cache.get(icon)
  if (!pending) {
    pending = getIcon(icon).catch(() => null)
    cache.set(icon, pending)
  }
  return pending
}

export function BrowserIcon({ icon }: { icon?: string }) {
  const [source, setSource] = useState<string | null>(null)

  useEffect(() => {
    let active = true
    setSource(null)
    if (icon) void loadIcon(icon).then((value) => { if (active) setSource(value) })
    return () => { active = false }
  }, [icon])

  return source
    ? <img className="browser-icon" src={source} alt="" />
    : <span className="browser-icon browser-icon-placeholder" aria-hidden="true">◎</span>
}
