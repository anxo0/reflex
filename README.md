# Reflex

<img src="assets/icon-256.png" width="96" alt="Icono de Reflex">

Duplica y controla la pantalla de tu móvil en el PC. **Libre, open source y sin límite de tiempo**: sin cuentas, sin anuncios, sin marcas de agua.

Escrito en Rust. Funciona en Windows, Linux y macOS.

| Plataforma | Estado |
|---|---|
| Android (USB y Wi-Fi) | ✅ Disponible |
| iPhone / iPad (AirPlay) | 🚧 En desarrollo, ver [docs/IPHONE.md](docs/IPHONE.md) |

## Funciones (Android)

- Duplicado de pantalla en tiempo real (H.264, hasta 120 fps)
- **Control total con ratón y teclado**: toca, arrastra, desplaza con la rueda y escribe
- **Audio del móvil** en el PC (Android 11+)
- Conexión por **cable USB** o **Wi-Fi** (botón «Pasar a Wi-Fi» o emparejado con código en Android 11+)
- Botones de Atrás, Inicio, Recientes, volumen, encendido, girar y notificaciones
- **Capturas** en PNG y **grabación** en MP4 (sin recomprimir, calidad original)
- Portapapeles compartido: Ctrl+C / Ctrl+V entre el PC y el móvil
- Apagar la pantalla del móvil sin dejar de duplicar
- Pantalla completa (F11)
- No hay que instalar nada en el móvil

## Uso

1. Descarga Reflex desde [Releases](../../releases) y ábrelo.
2. La primera vez pulsa **«Descargar componentes»** (adb de Google y el servidor de scrcpy, unos 15 MB).
3. En el móvil activa la **depuración USB**:
   *Ajustes → Información del teléfono →* pulsa 7 veces *Número de compilación*, y luego *Opciones de desarrollador → Depuración USB*.
4. Conecta el cable, acepta el aviso del móvil y pulsa **▶ Duplicar**.

### Sin cable

- **Pasar a Wi-Fi**: con el móvil conectado por USB, pulsa «Pasar a Wi-Fi» y después quita el cable. El móvil y el PC tienen que estar en la misma red.
- **Android 11+**: en *Opciones de desarrollador → Depuración inalámbrica → Vincular con código*, mete la IP:puerto y el código en «Emparejar». Luego conecta con la IP:puerto que aparece en esa misma pantalla.

### Controles

| Acción en el PC | En el móvil |
|---|---|
| Clic izquierdo / arrastrar | Tocar / deslizar |
| Clic derecho | Atrás (o encender la pantalla) |
| Clic central | Inicio |
| Rueda del ratón | Desplazar |
| Teclado | Escribir (haz clic antes en la pantalla) |
| Ctrl+V | Pegar el portapapeles del PC en el móvil |
| Ctrl+C | Copiar lo seleccionado en el móvil al PC |
| F11 / Esc | Pantalla completa / salir |

Las capturas se guardan en `Imágenes/Reflex` y las grabaciones en `Vídeos/Reflex`.

## Compilar

Necesitas [Rust](https://rustup.rs) (1.88 o superior) y un compilador de C/C++ (en Windows, las Build Tools de Visual Studio).

```sh
cargo build --release
```

El ejecutable queda en `target/release/reflex(.exe)`. En Linux hacen falta además las cabeceras de ALSA (`libasound2-dev`).

Pruebas: `cargo test` (algunas usan `ffmpeg` si está instalado para generar vídeo H.264 real).

## Cómo funciona

```
┌──────────── PC (Reflex, Rust) ────────────┐        ┌────── Android ──────┐
│ egui  ←  OpenH264  ←  socket vídeo  ←──────┼─ adb ──┼── scrcpy-server     │
│ cpal  ←  PCM 48 kHz ← socket audio  ←──────┼ túnel ─┼── (MediaCodec,      │
│ ratón/teclado → socket control  ───────────┼───────→┼──  AudioRecord,     │
└────────────────────────────────────────────┘        │    InputManager)    │
                                                      └─────────────────────┘
```

- `src/adb.rs`: lista de dispositivos, Wi-Fi, emparejado y túnel.
- `src/android/`: arranque del servidor y protocolo binario (vídeo, audio y control).
- `src/video.rs`: decodificación H.264 con OpenH264.
- `src/audio.rs`: reproducción del audio con cpal.
- `src/recorder.rs`: grabación MP4.
- `src/app.rs`: interfaz.

En el móvil se usa el servidor de [scrcpy](https://github.com/Genymobile/scrcpy) v5.0.1, que se descarga desde su release oficial y se comprueba con su SHA-256. La parte de PC (interfaz, decodificación, control, audio, grabación) es propia.

## Marca

Icono, colores, tipografía y voz de Reflex en el [manual de marca](docs/marca/README.md).

## Licencia

[GPL-3.0-or-later](LICENSE). Componentes de terceros en [THIRD_PARTY.md](THIRD_PARTY.md).

Reflex es un proyecto independiente: no está afiliado a Apowersoft, Apple, Google ni Genymobile.
