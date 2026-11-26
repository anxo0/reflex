# Instalador de Reflex

Instalador de Windows hecho con [Inno Setup 6](https://jrsoftware.org/isinfo.php). Sustituye el asistente por una ventana propia sin bordes de 760 x 460 con tres pantallas: bienvenida, instalando y listo.

## Qué hace

- Instala `ReflexMirror.exe`, `LICENSE`, `README.md` y `THIRD_PARTY.md` en `C:\Program Files\Reflex` (se puede cambiar) y pide permisos de administrador.
- Crea el acceso en el menú Inicio y, si se deja marcada la casilla, en el escritorio.
- Añade la regla de entrada «Reflex» al Firewall para redes privadas y públicas, después de borrar la que hubiera.
- Si se marca «Pantalla inalámbrica» (desmarcada de serie), la añade con DISM y deja `%LOCALAPPDATA%\Reflex\pantalla_inalambrica_instalada.txt` en la carpeta del usuario que abrió el instalador, que la app lee en `src/miracast.rs`. Si `netsh wlan show drivers` no encuentra un adaptador Wi-Fi que admita «Monitor inalámbrico», la casilla aparece desactivada con el aviso.
- Al desinstalar borra la regla del Firewall y conserva los datos de `%LOCALAPPDATA%\Reflex`.
- Español e inglés, según el idioma de Windows.

## Compilar

Necesita Inno Setup 6 (`winget install JRSoftware.InnoSetup`). Prepara una carpeta con lo que se instala y compila desde la raíz del repositorio:

```powershell
cargo build --release
New-Item -ItemType Directory -Force salida\origen | Out-Null
Copy-Item target\release\ReflexMirror.exe, LICENSE, README.md, THIRD_PARTY.md salida\origen
& "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" /DVersion=0.1.0 /DOrigen="$PWD\salida\origen" /O"$PWD\salida" installer\reflex.iss
```

Sale `salida\Reflex-0.1.0-setup.exe`. Si Inno Setup se instaló para todos los usuarios, `ISCC.exe` está en `C:\Program Files (x86)\Inno Setup 6\`. En GitHub Actions, con Inno Setup en el `PATH` (las imágenes de Windows de GitHub suelen traerlo; si no, `choco install innosetup -y`):

```powershell
iscc /DVersion=${{ env.VERSION }} /DOrigen="${{ github.workspace }}\salida\origen" /O"${{ github.workspace }}\salida" installer\reflex.iss
```

`/DOrigen` tiene que ser una ruta absoluta o relativa a `installer\`, porque Inno Setup resuelve las rutas desde la carpeta del `.iss`.

## Pantallas

Los fondos y las piezas (botones, casillas, barra) son BMP en `piezas/`, a 100, 125, 150 y 200 % de escala y en los dos idiomas, junto con `piezas/posiciones.iss`, que dice dónde va cada una. Están en el repositorio, así que compilar solo necesita Inno Setup.

Las pantallas muestran la versión, así que hay que volver a dibujarlas al cambiarla o al tocar el diseño. ISCC se niega a compilar si no coinciden.

```powershell
installer\construir.ps1 -Version 0.1.0
```

Necesita Python, Node 22 o posterior, Microsoft Edge e internet (Sora e Inter se cargan de Google Fonts). `diseno/generar.py` escribe las pantallas en HTML con los colores y fuentes de `docs/marca` y `diseno/piezas.mjs` las captura con Edge sin ventana.

## Probar

```powershell
salida\Reflex-0.1.0-setup.exe
netsh advfirewall firewall show rule name=Reflex
```

La instalación deja su registro en `%TEMP%\Setup Log *.txt` si se abre con `/LOG`.
