# Componentes de terceros

## Descargados en tiempo de ejecución (no se incluyen en el repositorio)

| Componente | Licencia | Origen |
|---|---|---|
| scrcpy-server v5.0.1 | Apache License 2.0 | https://github.com/Genymobile/scrcpy (Copyright © 2018 Genymobile, © 2018-2026 Romain Vimont) |
| Android SDK Platform-Tools (adb) | Android Software Development Kit License | https://developer.android.com/tools/releases/platform-tools |

El servidor de scrcpy se descarga de la release oficial y se verifica con el SHA-256 publicado (`764eb6f79811d5211fe9df341120882ba9994c7a61b897d7bf3fb662e53bc536`).

## Código derivado incluido en el repositorio

| Componente | Licencia | Origen | Dónde |
|---|---|---|---|
| playfair (FairPlay SAP v3) | GPL-3.0 | https://github.com/EstebanKubata/playfair, vía [UxPlay](https://github.com/FDH2/UxPlay) | `src/iphone/fairplay/`: port a Rust; `tables.rs` se genera a partir de sus fuentes |
| Decodificador AAC de FFmpeg (perfil ELD) | LGPL-2.1+ (usado aquí bajo GPL-3.0, como permite su sección 3) | https://github.com/FFmpeg/FFmpeg (`libavcodec/aac/`, `aactab.c`) | `src/iphone/aac/`: port a Rust; `tables.rs` se genera a partir de `aactab.c` |

El receptor AirPlay (`src/iphone/`) se ha escrito tomando como referencia el comportamiento de UxPlay (GPL-3.0), RPiPlay (GPL-3.0) y shairplay (LGPL-2.1+). Salvo los ports de playfair y del decodificador AAC de FFmpeg, el código es propio.

Las muestras de `src/iphone/aac/testdata/` son una señal de prueba sintética codificada y decodificada con [fdk-aac](https://github.com/mstorsjo/fdk-aac). fdk-aac solo se usó para generarlas: no se incluye ni se enlaza en Reflex.

## Bibliotecas enlazadas

| Biblioteca | Licencia |
|---|---|
| OpenH264 (vía crate `openh264`) | BSD 2-Clause, © Cisco Systems |
| egui / eframe | MIT o Apache-2.0 |
| cpal | Apache-2.0 |
| mp4 | MIT |
| plist | MIT |
| ed25519-dalek, x25519-dalek | BSD 3-Clause |
| image, ureq, zip, sha2, aes, ctr, getrandom, mdns-sd, dirs, chrono, anyhow, bytes | MIT o Apache-2.0 |

Nota sobre H.264: OpenH264 se compila desde el código fuente. La licencia de patentes de H.264 que paga Cisco solo cubre sus binarios oficiales; quien distribuya Reflex en países con patentes de software debe tenerlo en cuenta.
