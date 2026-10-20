// Driven by profiles_smoke.py --browser. Uses the system Chromium and Node's CDP WebSocket.
import { spawn } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import { setTimeout as delay } from 'node:timers/promises'
import assert from 'node:assert/strict'

const [base, directory] = process.argv.slice(2)
await mkdir(`${directory}/chromium`, { recursive: true })
const chromium = spawn('chromium', ['--headless', '--no-sandbox', '--disable-gpu', '--no-first-run',
  '--disable-dev-shm-usage', '--remote-debugging-port=0', `--user-data-dir=${directory}/chromium`,
  '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1, EXCLUDE localhost', 'about:blank'],
{ stdio: ['ignore', 'ignore', 'pipe'] })
let socket
try {
  const endpoint = await new Promise((resolve, reject) => {
    let output = ''
    const timer = setTimeout(() => reject(new Error('Chromium did not start')), 10000)
    chromium.stderr.on('data', (data) => {
      output += data.toString()
      const match = output.match(/DevTools listening on ws:\/\/(127\.0\.0\.1:\d+)/)
      if (match) { clearTimeout(timer); resolve(`http://${match[1]}`) }
    })
    chromium.once('exit', (code) => { clearTimeout(timer); reject(new Error(`Chromium exited: ${code}`)) })
  })
  const pages = await (await fetch(`${endpoint}/json/list`)).json()
  const page = pages.find((item) => item.type === 'page')
  socket = new WebSocket(page.webSocketDebuggerUrl)
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject })
  let nextId = 0
  const pending = new Map()
  const errors = []
  socket.onmessage = ({ data }) => {
    const message = JSON.parse(data)
    if (message.method === 'Runtime.exceptionThrown') errors.push(message.params)
    const callbacks = pending.get(message.id)
    if (callbacks) { pending.delete(message.id); message.error ? callbacks.reject(message.error) : callbacks.resolve(message.result) }
  }
  async function send(method, params = {}) {
    const id = ++nextId
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timed out: ${method}`)) }, 5000)
      pending.set(id, { resolve: (value) => { clearTimeout(timer); resolve(value) }, reject: (error) => { clearTimeout(timer); reject(error) } })
      socket.send(JSON.stringify({ id, method, params }))
    })
  }
  async function evaluate(expression) {
    const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails))
    return result.result.value
  }
  async function wait(expression) {
    const deadline = Date.now() + 8000
    while (Date.now() < deadline) {
      if (await evaluate(`Boolean(${expression})`)) return
      await delay(50)
    }
    throw new Error(`UI condition failed: ${expression}\n${await evaluate('document.body.innerText')}`)
  }
  async function key(value) {
    // Let React finish the screen transition and install its keyboard handlers.
    await evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))).then(() => true)')
    const code = { Enter: 13, Escape: 27, ArrowDown: 40, ArrowRight: 39, ArrowUp: 38, ArrowLeft: 37, p: 80, d: 68 }[value]
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key: value, windowsVirtualKeyCode: code, text: value === 'Enter' ? String.fromCharCode(13) : undefined })
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key: value, windowsVirtualKeyCode: code })
  }
  await send('Runtime.enable')
  await send('Page.enable')
  await send('Emulation.setDeviceMetricsOverride', { width: 1920, height: 1080, deviceScaleFactor: 1, mobile: false })
  await send('Page.navigate', { url: base })
  await wait("document.querySelector('.profile-card') && document.activeElement?.textContent.includes('Guest')")
  await key('Enter')
  await wait("document.querySelector('.profile-label')?.textContent === 'Guest'")
  await key('p')
  await wait("document.querySelector('.profile-grid')")
  await key('ArrowDown') // Last card is Add profile.
  await key('Enter')
  await wait("document.activeElement?.tagName === 'INPUT'")
  await send('Input.insertText', { text: 'Keyboard test' })
  await key('Enter')
  await wait("document.querySelector('.page-title')?.textContent.includes('Connect Keyboard test')")
  const profiles = await (await fetch(`${base}/api/profiles`)).json()
  const profile = profiles.profiles.find((item) => item.name === 'Keyboard test')
  assert(profile)
  await writeFile(`${directory}/close-${profile.id}`, '')
  await wait("document.body.innerText.includes('YouTube sign-in works')")
  await wait("document.activeElement?.textContent === 'Continue to CouchTube'")
  await key('Enter')
  await wait("document.querySelector('.profile-label')?.textContent === 'Keyboard test' && document.querySelector('.card .card-title')?.textContent === 'Fixture video'")
  await key('ArrowDown') // Home menu.
  await key('ArrowDown') // Recommended (empty Continue row is skipped).
  await wait("document.querySelector('.card.focused .card-title')")
  const selected = await evaluate("document.querySelector('.card.focused .card-title')?.textContent")
  assert(selected)
  await key('Enter')
  await wait("document.querySelector('.toast')?.textContent.includes('Playing')")
  await key('Escape')
  await wait("!document.querySelector('.toast')")
  assert.equal(await evaluate("document.querySelector('.card.focused .card-title')?.textContent"), selected)
  await key('a')
  await wait("document.querySelector('.toast')?.textContent.includes('Queued:')")
  await key('ArrowUp') // Home menu, skipping the empty Continue row.
  for (const label of ['History', 'Watch later', 'Liked videos']) {
    await key('Enter')
    await wait(`document.querySelector('.page-title')?.textContent === ${JSON.stringify(label)}`)
    await wait("document.querySelector('.card .card-title')?.textContent === 'Fixture video'")
    await key('Escape')
    await wait("document.querySelector('.page-title')?.textContent === 'What do you want to watch?'")
    await key('ArrowRight')
  }
  await key('Enter') // Queue.
  await wait("document.querySelector('.page-title')?.textContent === 'Up next'")
  await wait("document.querySelector('.card .card-title')?.textContent === 'Fixture video'")
  await key('ArrowDown')
  await key('d')
  await wait("document.body.innerText.includes('Your queue is empty.')")
  await key('Escape')
  await key('ArrowDown') // Back to Recommended.
  await wait("document.querySelector('.card.focused')")
  await key('/')
  await wait("document.activeElement?.tagName === 'INPUT'")
  await send('Input.insertText', { text: 'focus regression' })
  await key('Enter')
  await wait("document.querySelector('.card.focused .card-title')?.textContent === 'Fixture video'")
  await key('Escape')
  await wait("document.querySelector('.card.focused')?.closest('section')?.querySelector('h2')?.textContent === 'Recommended'")
  await key('p')
  await wait("document.querySelector('.profile-grid')")
  await evaluate("[...document.querySelectorAll('.profile-card')].find(e => e.querySelector('strong')?.textContent === 'Keyboard test').focus()")
  await key('d')
  await wait("document.querySelector('.page-title')?.textContent === 'Remove Keyboard test?'")
  await key('ArrowDown')
  await key('Enter')
  await wait("document.querySelector('.profile-grid') && !document.body.innerText.includes('Keyboard test')")
  await key('ArrowRight') // Guest -> Bob.
  await key('Enter')
  await wait("document.querySelector('.profile-label')?.textContent === 'Bob'")
  assert.deepEqual(errors, [], 'browser raised a JavaScript exception')
  console.log('Keyboard browser check passed: Guest, P, add/sign-in, continue, playback/return, feeds, queue, recent-search focus, removal, profile switch.')
} finally {
  socket?.close()
  chromium.kill('SIGTERM')
  await new Promise((resolve) => { if (chromium.exitCode !== null) resolve(); else chromium.once('exit', resolve) })
}
