"""Builds the installer screens (760x460) as one HTML page; piezas.mjs renders them to images.

Every screen exists in Spanish and English. Elements marked data-pieza become separate
images laid over the background by reflex.iss; data-texto marks where the installer
writes live text (folder, progress, notes). Both only report their position.

Usage: python generar.py <version>
"""
import re
import sys
from pathlib import Path

version = sys.argv[1]

aqui = Path(__file__).parent
raiz = aqui.parent.parent
salida = aqui / 'construido'
salida.mkdir(exist_ok=True)

icono = (raiz / 'docs' / 'marca' / 'logos' / 'reflex-icono.svg').read_text(encoding='utf-8')
icono = re.sub(r'<\?xml[^>]*>', '', icono)
icono = re.sub(r'<svg ', '<svg class="icono" aria-hidden="true" ', icono, count=1)

TEXTOS = {
    'es': {
        'bienvenida_h1': 'El móvil, en tu PC.',
        'bienvenida_sub': 'Duplica y controla tu Android o tu iPhone desde el escritorio. Libre, open source y sin límite de tiempo.',
        'carpeta': 'Se instala en',
        'cambiar': 'Cambiar',
        'escritorio': 'Acceso directo en el escritorio',
        'inalambrica': 'Pantalla inalámbrica',
        'inalambrica_detalle': 'Para «Enviar pantalla» desde Android',
        'sin_wifi': 'Este PC no tiene Wi-Fi: «Enviar pantalla» no funcionará',
        'version': f'Versión {version}',
        'cancelar': 'Cancelar',
        'instalar': 'Instalar',
        'instalando_h1': 'Instalando Reflex',
        'instalando_sub': 'Tarda menos de un minuto. Si añadiste Pantalla inalámbrica, Windows la descarga y puede llevar unos minutos más.',
        'listo_h1': 'Reflex está listo.',
        'listo_sub': 'Lo tienes en el menú Inicio. Conecta el móvil por USB o por Wi-Fi y empieza a duplicar.',
        'abrir': 'Abrir Reflex',
        'terminar': 'Terminar',
    },
    'en': {
        'bienvenida_h1': 'Your phone, on your PC.',
        'bienvenida_sub': 'Mirror and control your Android or iPhone from the desktop. Free, open source and with no time limit.',
        'carpeta': 'Installs to',
        'cambiar': 'Change',
        'escritorio': 'Desktop shortcut',
        'inalambrica': 'Wireless Display',
        'inalambrica_detalle': 'For screen cast from Android',
        'sin_wifi': 'This PC has no Wi-Fi: screen cast will not work',
        'version': f'Version {version}',
        'cancelar': 'Cancel',
        'instalar': 'Install',
        'instalando_h1': 'Installing Reflex',
        'instalando_sub': 'It takes under a minute. If you added Wireless Display, Windows downloads it and it can take a few more minutes.',
        'listo_h1': 'Reflex is ready.',
        'listo_sub': 'You will find it in the Start menu. Connect your phone by USB or Wi-Fi and start mirroring.',
        'abrir': 'Open Reflex',
        'terminar': 'Finish',
    },
}

CAJA = '<span class="caja"><svg viewBox="0 0 12 12"><path d="M2.4 6.3l2.3 2.3 4.9-5.2"/></svg></span>'


def unico(svg, sufijo):
    """Gives the SVG's gradients ids of their own: hidden windows would otherwise lend theirs to the rest."""
    for nombre in set(re.findall(r'id="([^"]+)"', svg)):
        svg = svg.replace(f'id="{nombre}"', f'id="{nombre}-{sufijo}"').replace(f'url(#{nombre})', f'url(#{nombre}-{sufijo})')
    return svg


def movil(lleno):
    """The brand mark at panel scale: the phone rises from the bottom edge, its reflection behind."""
    return f'''
<svg class="movil{' lleno' if lleno else ''}" viewBox="0 0 304 460" aria-hidden="true">
  <defs><linearGradient id="reflejo" x1="0" y1="0" x2="1" y2="0">
    <stop offset="0" stop-color="#A9BFFF"/><stop offset="1" stop-color="#A9BFFF" stop-opacity="0.18"/>
  </linearGradient></defs>
  <rect class="reflejo" x="144" y="178" width="152" height="310" rx="37" fill="url(#reflejo)"/>
  <rect x="52" y="178" width="152" height="310" rx="37" fill="#FFFFFF"/>
  <rect x="106" y="201" width="44" height="12" rx="6" fill="#001489"/>
</svg>'''


def ventana(pantalla, lang, izquierda, lleno=False):
    return f'''
<section class="ventana" id="{pantalla}-{lang}" data-pantalla="{pantalla}" data-lang="{lang}">
  <div class="izquierda">
    <header><span class="marca">{unico(icono, pantalla + lang)}Reflex</span></header>
    {izquierda}
  </div>
  <div class="derecha">
    {unico(movil(lleno), pantalla + lang)}
    <span class="cerrar" data-pieza="cerrar" data-comun data-variantes="hover"><svg viewBox="0 0 12 12"><path d="M2 2l8 8M10 2l-8 8"/></svg></span>
  </div>
</section>'''


def pantallas(lang):
    t = TEXTOS[lang]
    bienvenida = ventana('bienvenida', lang, f'''
    <h1>{t['bienvenida_h1']}</h1>
    <p class="sub">{t['bienvenida_sub']}</p>
    <p class="etiqueta">{t['carpeta']}</p>
    <div class="campo">
      <span class="ruta" data-texto="ruta">C:\\Program Files\\Reflex</span>
      <span class="enlace" data-pieza="cambiar" data-variantes="hover">{t['cambiar']}</span>
    </div>
    <div class="casilla" data-pieza="escritorio" data-variantes="no">
      <span class="fila">{CAJA}<span class="rotulo">{t['escritorio']}</span></span>
    </div>
    <div class="casilla doble" data-pieza="inalambrica" data-variantes="no,off">
      <span class="fila">{CAJA}<span class="rotulo">{t['inalambrica']}</span></span>
      <span class="notas"><span class="detalle">{t['inalambrica_detalle']}</span><span class="nota">{t['sin_wifi']}</span></span>
    </div>
    <footer>
      <span class="version">{t['version']}</span>
      <span class="boton secundario" data-pieza="cancelar" data-variantes="hover">{t['cancelar']}</span>
      <span class="boton principal" data-pieza="instalar" data-variantes="hover">{t['instalar']}</span>
    </footer>''')

    instalando = ventana('instalando', lang, f'''
    <h1>{t['instalando_h1']}</h1>
    <p class="sub">{t['instalando_sub']}</p>
    <div class="progreso">
      <div class="linea"><span class="estado" data-texto="estado">Copiando archivos</span><span class="cifra" data-texto="cifra">62 %</span></div>
      <div class="pista"><div class="relleno" data-pieza="relleno" data-comun></div></div>
    </div>
    <footer>
      <span class="boton secundario solo" data-pieza="cancelar-instalando" data-imagen="cancelar">{t['cancelar']}</span>
    </footer>''')

    listo = ventana('listo', lang, f'''
    <h1>{t['listo_h1']}</h1>
    <p class="sub">{t['listo_sub']}</p>
    <p class="aviso" data-texto="aviso">Reinicia el PC para que «Enviar pantalla» funcione.</p>
    <footer>
      <div class="casilla pie" data-pieza="abrir" data-variantes="no">
        <span class="fila">{CAJA}<span class="rotulo">{t['abrir']}</span></span>
      </div>
      <span class="boton principal" data-pieza="terminar" data-variantes="hover">{t['terminar']}</span>
    </footer>''', lleno=True)
    return bienvenida + instalando + listo


html = f'''<!doctype html>
<html lang="es"><head><meta charset="utf-8"><title>Instalador de Reflex</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=Sora:wght@600&display=block">
<style>
:root {{
  --surface: #0B0E1A; --raised: #151A2E; --line: #262D47; --ink: #EEF2FF; --muted: #A3ABC8;
  --quiet: #8A93B2; --off: #5D6584; --blue: #001489; --blue-bright: #2447E0; --blue-hover: #3A5CF0;
  --link: #7088FF; --ice: #A9BFFF;
}}
* {{ box-sizing: border-box; margin: 0; }}
body {{ background: #1C2033; font-family: Inter, sans-serif; color: var(--ink); display: grid; gap: 40px;
  padding: 40px; justify-content: center; -webkit-font-smoothing: antialiased; font-feature-settings: 'cv11', 'ss03'; }}
.ventana {{ width: 760px; height: 460px; display: grid; grid-template-columns: 456px 304px; background: var(--surface);
  border-radius: 8px; overflow: hidden; }}
.izquierda {{ position: relative; padding: 0 40px; }}
header {{ position: absolute; top: 24px; left: 40px; height: 24px; display: flex; align-items: center; }}
.marca {{ display: flex; align-items: center; gap: 9px; font-family: Sora, sans-serif; font-weight: 600; font-size: 15px; letter-spacing: -0.01em; }}
.icono {{ width: 22px; height: 22px; }}
h1 {{ margin-top: 84px; font-family: Sora, sans-serif; font-weight: 600; font-size: 29px; line-height: 36px; letter-spacing: -0.022em; }}
.sub {{ margin-top: 10px; font-size: 14.5px; line-height: 22px; color: var(--muted); max-width: 360px; }}
.etiqueta {{ margin-top: 26px; font-size: 12px; line-height: 16px; font-weight: 500; color: var(--quiet); }}
.campo {{ margin-top: 8px; height: 38px; display: flex; align-items: center; justify-content: space-between; gap: 12px;
  padding: 0 4px 0 14px; border-radius: 6px; background: var(--raised); box-shadow: inset 0 0 0 1px var(--line); }}
.ruta {{ flex: 1; font-family: 'Segoe UI Variable Text', 'Segoe UI', sans-serif; font-size: 13px; line-height: 20px;
  white-space: nowrap; overflow: hidden; }}
.enlace {{ display: grid; place-items: center; width: 72px; height: 30px; border-radius: 5px; font-size: 13px; font-weight: 600; color: var(--link); }}
.enlace.hover {{ background: #1E2540; color: #93A5FF; }}
.casilla {{ width: 376px; }}
.campo + .casilla {{ margin-top: 18px; }}
.casilla + .casilla {{ margin-top: 10px; }}
.casilla .fila {{ display: flex; align-items: center; gap: 11px; height: 22px; font-size: 13.5px; font-weight: 500; }}
.casilla.doble {{ height: 42px; }}
.caja {{ flex: none; display: grid; place-items: center; width: 18px; height: 18px; border-radius: 5px; background: var(--blue-bright); }}
.caja svg {{ width: 12px; height: 12px; fill: none; stroke: #FFFFFF; stroke-width: 1.9; stroke-linecap: round; stroke-linejoin: round; }}
.casilla.no .caja, .casilla.off .caja {{ background: transparent; box-shadow: inset 0 0 0 1.5px #3B4468; }}
.casilla.no .caja svg, .casilla.off .caja svg {{ visibility: hidden; }}
.casilla.off .caja {{ box-shadow: inset 0 0 0 1.5px var(--line); }}
.casilla.off .rotulo {{ color: var(--off); }}
.notas {{ display: grid; margin: 2px 0 0 29px; font-size: 12px; line-height: 16px; color: var(--muted); }}
.detalle, .nota {{ grid-area: 1 / 1; }}
.nota {{ visibility: hidden; }}
.casilla.off .nota {{ visibility: visible; }}
.casilla.off .detalle {{ visibility: hidden; }}
footer {{ position: absolute; left: 40px; right: 40px; bottom: 28px; height: 40px; display: flex; align-items: center; gap: 8px; }}
.version {{ margin-right: auto; font-size: 12px; color: var(--quiet); font-variant-numeric: tabular-nums; }}
.boton {{ display: grid; place-items: center; height: 40px; border-radius: 10px; font-size: 14px; font-weight: 600; }}
.principal {{ width: 128px; background: var(--blue-bright); color: #FFFFFF; }}
.principal.hover {{ background: var(--blue-hover); }}
.secundario {{ width: 104px; color: var(--ink); box-shadow: inset 0 0 0 1px var(--line); }}
.secundario.hover {{ background: var(--raised); box-shadow: inset 0 0 0 1px #3B4468; }}
.solo {{ margin-left: auto; }}
.casilla.pie {{ width: 180px; margin: 0 auto 0 0; }}
.progreso {{ margin-top: 34px; }}
.linea {{ display: flex; justify-content: space-between; align-items: baseline; height: 20px;
  font-family: 'Segoe UI Variable Text', 'Segoe UI', sans-serif; font-size: 13px; color: var(--muted); }}
.cifra {{ width: 60px; text-align: right; color: var(--ink); font-weight: 600; }}
.pista {{ margin-top: 10px; height: 6px; border-radius: 3px; background: var(--line); overflow: hidden; }}
.relleno {{ width: 62%; height: 100%; border-radius: 3px; background: var(--blue-bright); }}
.aviso {{ margin-top: 18px; height: 20px; font-family: 'Segoe UI Variable Text', 'Segoe UI', sans-serif; font-size: 13px; color: var(--ice); }}
.derecha {{ position: relative; background: var(--blue); }}
.movil {{ position: absolute; inset: 0; width: 304px; height: 460px; }}
.movil .reflejo {{ opacity: .42; }}
.movil.lleno .reflejo {{ opacity: 1; }}
.cerrar {{ position: absolute; top: 14px; right: 14px; display: grid; place-items: center; width: 32px; height: 32px; border-radius: 8px; }}
.cerrar svg {{ width: 11px; height: 11px; fill: none; stroke: var(--ice); stroke-width: 1.5; stroke-linecap: round; }}
.cerrar.hover {{ background: rgba(255,255,255,.12); }}
.cerrar.hover svg {{ stroke: #FFFFFF; }}
.fondo [data-pieza], .fondo [data-texto] {{ visibility: hidden !important; }}
.fondo .pista [data-pieza] {{ visibility: hidden !important; }}
.captura {{ padding: 0; display: block; background: var(--surface); }}
.captura .ventana {{ display: none; border-radius: 0; }}
.captura .ventana.activa {{ display: grid; }}
</style></head><body>
{pantallas('es')}
{pantallas('en')}
</body></html>'''
(salida / 'index.html').write_text(html, encoding='utf-8')
print('OK')
