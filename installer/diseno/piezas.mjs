// Renders every installer piece and background at each Windows scale through Edge's DevTools protocol,
// and writes posiciones.iss with where each piece and live text sits, in pixels at 100 %.
// Usage: node piezas.mjs <msedge.exe> <construido folder>
import { spawn } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const [edge, carpeta] = process.argv.slice(2)
const salida = join(carpeta, 'piezas')
mkdirSync(salida, { recursive: true })
const url = 'file:///' + carpeta.replaceAll('\\', '/') + '/index.html'
const puerto = 9341
const proceso = spawn(edge, ['--headless=new', '--disable-gpu', '--hide-scrollbars', '--force-color-profile=srgb',
  '--font-render-hinting=none', `--remote-debugging-port=${puerto}`, `--user-data-dir=${join(carpeta, 'perfil-edge')}`,
  'about:blank'], { stdio: 'ignore' })

const espera = (ms) => new Promise((r) => setTimeout(r, ms))
let objetivos
for (let i = 0; i < 60 && !objetivos; i++) {
  await espera(250)
  try { objetivos = await (await fetch(`http://127.0.0.1:${puerto}/json`)).json() } catch {}
}
if (!objetivos) throw new Error('Edge no ha arrancado')
const ws = new WebSocket(objetivos.find((t) => t.type === 'page').webSocketDebuggerUrl)
await new Promise((r) => (ws.onopen = r))
let id = 0
const pendientes = new Map()
ws.onmessage = (e) => {
  const m = JSON.parse(e.data)
  if (pendientes.has(m.id)) { pendientes.get(m.id)(m); pendientes.delete(m.id) }
}
const cdp = (method, params = {}) => new Promise((r, f) => {
  const n = ++id
  pendientes.set(n, (m) => (m.error ? f(new Error(`${method}: ${m.error.message}`)) : r(m.result)))
  ws.send(JSON.stringify({ id: n, method, params }))
})
const evaluar = async (expression) =>
  (await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })).result.value

const captura = async (nombre, r) => {
  const res = await cdp('Page.captureScreenshot', {
    format: 'png', clip: { x: r.x, y: r.y, width: r.w, height: r.h, scale: 1 },
  })
  writeFileSync(join(salida, nombre + '.png'), Buffer.from(res.data, 'base64'))
}

await cdp('Page.enable')
const posiciones = {}
let fuentes = []
for (const escala of [100, 125, 150, 200]) {
  // One window at a time, at its own size: a tall page at 200 % stalls Edge's capture.
  await cdp('Emulation.setDeviceMetricsOverride', { width: 760, height: 460, deviceScaleFactor: escala / 100, mobile: false })
  await cdp('Page.navigate', { url })
  await espera(1200)
  fuentes = await evaluar(`document.fonts.ready.then(() => [...document.fonts].filter(f => f.status === 'loaded').map(f => f.family + ' ' + f.weight))`)
  if (!fuentes.some((f) => f.includes('Sora')) || !fuentes.some((f) => f.includes('Inter'))) {
    throw new Error('No han cargado Sora e Inter desde Google Fonts: ' + JSON.stringify(fuentes))
  }
  await evaluar(`document.body.classList.add('captura'); document.querySelectorAll('.relleno').forEach(r => (r.style.width = '100%'))`)
  const ids = await evaluar(`[...document.querySelectorAll('.ventana')].map(v => v.id)`)
  const hechas = new Set()
  for (const ventana of ids) {
    await evaluar(`document.body.classList.remove('fondo'); document.querySelectorAll('.ventana').forEach(v => v.classList.toggle('activa', v.id === '${ventana}'))`)
    await espera(60)
    // Every piece and live text of this window, with its box.
    const elementos = await evaluar(`[...document.querySelectorAll('#${ventana} [data-pieza], #${ventana} [data-texto]')].map(e => {
      const r = e.getBoundingClientRect()
      return { pieza: e.dataset.pieza, texto: e.dataset.texto, imagen: e.dataset.imagen || e.dataset.pieza,
        comun: e.hasAttribute('data-comun'), variantes: (e.dataset.variantes || '').split(',').filter(Boolean),
        lang: e.closest('.ventana').dataset.lang, x: r.x, y: r.y, w: r.width, h: r.height }
    })`)
    if (escala === 100) {
      for (const e of elementos) {
        const clave = e.pieza || e.texto
        const caja = { x: Math.round(e.x), y: Math.round(e.y), w: Math.round(e.w), h: Math.round(e.h) }
        const antes = posiciones[clave]
        if (antes && JSON.stringify(antes) !== JSON.stringify(caja)) {
          throw new Error(`${clave} no ocupa lo mismo en todas las pantallas o idiomas: ${JSON.stringify(antes)} y ${JSON.stringify(caja)}`)
        }
        posiciones[clave] ??= caja
      }
    }
    for (const e of elementos.filter((q) => q.pieza)) {
      const sufijo = (e.comun ? '' : '-' + e.lang) + '-' + escala
      if (hechas.has(e.imagen + sufijo)) continue
      hechas.add(e.imagen + sufijo)
      const selector = `#${ventana} [data-pieza="${e.pieza}"]`
      let caja = e
      if (e.pieza === 'relleno') {
        // The fill is drawn full width; the installer reveals as much of it as it needs.
        caja = await evaluar(`(() => { const el = document.querySelector('${selector}'); el.style.width = '100%'
          const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height } })()`)
        posiciones.relleno.w = Math.round(caja.w)
      }
      await captura(e.imagen + sufijo, caja)
      for (const v of e.variantes) {
        await evaluar(`document.querySelector('${selector}').classList.add('${v}')`)
        await espera(30)
        await captura(`${e.imagen}-${v}${sufijo}`, caja)
        await evaluar(`document.querySelector('${selector}').classList.remove('${v}')`)
      }
    }
    await evaluar(`document.body.classList.add('fondo')`)
    await espera(60)
    await captura(`fondo-${ventana}-${escala}`, { x: 0, y: 0, w: 760, h: 460 })
  }
}

const lineas = Object.entries(posiciones).flatMap(([clave, c]) => {
  const n = clave.replaceAll('-', '_')
  return [`#define ${n}_X ${c.x}`, `#define ${n}_Y ${c.y}`, `#define ${n}_W ${c.w}`, `#define ${n}_H ${c.h}`]
})
writeFileSync(join(carpeta, 'posiciones.iss'), lineas.join('\r\n') + '\r\n')
ws.close()
proceso.kill()
console.log('Fuentes:', [...new Set(fuentes)].join(', '))
console.log('Piezas:', Object.keys(posiciones).join(', '))
