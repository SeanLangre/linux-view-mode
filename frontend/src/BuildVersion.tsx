import { useEffect, useState } from 'react'
import * as api from './api'

/** The running build, small in the bottom-right corner: package version and git commit. */
export default function BuildVersion() {
  const [build, setBuild] = useState<api.BuildVersion | null>(null)
  useEffect(() => {
    let cancelled = false
    void api.version().then((value) => { if (!cancelled) setBuild(value) })
    return () => { cancelled = true }
  }, [])
  if (!build) return null
  const dirty = build.commit.endsWith('-dirty')
  return <div className="build-version" title={`CouchTube ${build.version}, commit ${build.commit.replace('-dirty', '')}${dirty ? ' (with uncommitted changes)' : ''}`}>
    v{build.version} · {build.commit}
  </div>
}
