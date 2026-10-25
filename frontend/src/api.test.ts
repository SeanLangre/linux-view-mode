import { afterEach, describe, expect, it, vi } from 'vitest'
import { play, playerStatus, stop } from './api'

afterEach(() => vi.unstubAllGlobals())

describe('playback API', () => {
  it('posts the video and resume position, and posts stop', async () => {
    const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) })
    vi.stubGlobal('fetch', fetcher)
    await play('aqz-KE-bpKQ', 12.5)
    await stop()
    expect(fetcher.mock.calls).toEqual([
      ['/api/play', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: 'aqz-KE-bpKQ', start: 12.5 }) }],
      ['/api/stop', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }],
    ])
  })

  it('always fetches fresh status and reports API errors', async () => {
    const fetcher = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: async () => ({ playing: true }) })
      .mockResolvedValueOnce({ ok: true, json: async () => ({ playing: false }) })
      .mockResolvedValueOnce({ ok: false, json: async () => ({ error: 'mpv unavailable' }) })
    vi.stubGlobal('fetch', fetcher)
    expect(await playerStatus()).toEqual({ playing: true })
    expect(await playerStatus()).toEqual({ playing: false })
    expect(fetcher.mock.calls[0]).toEqual(['/api/status', { cache: 'no-store' }])
    await expect(play('aqz-KE-bpKQ')).rejects.toThrow('mpv unavailable')
  })
})

// Account switching must not reuse another person's searches.
describe('profile cache', () => {
  it('drops in-flight browser cache entries when changing profiles', async () => {
    const { search, setProfileScope } = await import('./api')
    let finishAlice!: (value: unknown) => void
    const fetcher = vi.fn()
      .mockImplementationOnce(() => new Promise((resolve) => { finishAlice = resolve }))
      .mockResolvedValueOnce({ ok: true, json: async () => [{ id: 'bob-result' }] })
      .mockResolvedValueOnce({ ok: true, json: async () => [{ id: 'alice-fresh-result' }] })
    vi.stubGlobal('fetch', fetcher)
    setProfileScope('alice')
    const alice = search('isolated-cache-test')
    setProfileScope('bob')
    const bob = await search('isolated-cache-test')
    finishAlice({ ok: true, json: async () => [{ id: 'alice-result' }] })
    expect(await alice).toEqual([{ id: 'alice-result' }])
    expect(bob).toEqual([{ id: 'bob-result' }])
    expect(await search('isolated-cache-test')).toEqual(bob)
    setProfileScope('alice')
    expect(await search('isolated-cache-test')).toEqual([{ id: 'alice-fresh-result' }])
    expect(fetcher).toHaveBeenCalledTimes(3)
    setProfileScope('guest')
  })
})


describe('feeds and queue API', () => {
  it('does not cache feeds or queue state in the browser and posts queue actions', async () => {
    const { feed, queue, enqueue, next, deleteQueue } = await import('./api')
    const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => [] })
    vi.stubGlobal('fetch', fetcher)
    await feed('history')
    await feed('history', true)
    await queue()
    await enqueue('aqz-KE-bpKQ')
    await next()
    await deleteQueue('aqz-KE-bpKQ')
    await deleteQueue()
    expect(fetcher.mock.calls[0]).toEqual(['/api/feed/history?page=0', { cache: 'no-store' }])
    expect(fetcher.mock.calls[1]).toEqual(['/api/feed/history?page=0&refresh=true', { cache: 'no-store' }])
    expect(fetcher.mock.calls[2]).toEqual(['/api/queue', { cache: 'no-store' }])
    expect(fetcher.mock.calls[3][1].body).toBe(JSON.stringify({ id: 'aqz-KE-bpKQ' }))
    expect(fetcher.mock.calls[4][0]).toBe('/api/next')
    expect(fetcher.mock.calls[5]).toEqual(['/api/queue/aqz-KE-bpKQ', { method: 'DELETE' }])
    expect(fetcher.mock.calls[6]).toEqual(['/api/queue', { method: 'DELETE' }])
  })
})
