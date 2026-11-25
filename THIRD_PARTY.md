# Componentes de terceros

## Descargados en tiempo de ejecución (no se incluyen en el repositorio)

| Componente | Licencia | Origen |
|---|---|---|
| scrcpy-server v5.0.1 | Apache License 2.0 | https://github.com/Genymobile/scrcpy (Copyright © 2018 Genymobile, © 2018-2026 Romain Vimont) |
| Android SDK Platform-Tools (adb) | Android Software Development Kit License | https://developer.android.com/tools/releases/platform-tools |

El servidor de scrcpy se descarga de la release oficial y se verifica con el SHA-256 publicado (`764eb6f79811d5211fe9df341120882ba9994c7a61b897d7bf3fb662e53bc536`).

## Bibliotecas enlazadas

| Biblioteca | Licencia |
|---|---|
| OpenH264 (vía crate `openh264`) | BSD 2-Clause, © Cisco Systems |
| egui / eframe | MIT o Apache-2.0 |
| cpal | Apache-2.0 |
| mp4 | MIT |
| image, ureq, zip, sha2, dirs, chrono, anyhow, bytes | MIT o Apache-2.0 |

Nota sobre H.264: OpenH264 se compila desde el código fuente. La licencia de patentes de H.264 que paga Cisco solo cubre sus binarios oficiales; quien distribuya Reflex en países con patentes de software debe tenerlo en cuenta.
