# iPhone / iPad (fase 2)

## Por qué AirPlay

En Windows no hay forma pública de capturar la pantalla de un iPhone por cable (en macOS sí existe, vía QuickTime). Las apps de duplicado para iPhone usan **AirPlay**: el PC se anuncia en la red como un Apple TV y el iPhone le envía su pantalla desde *Centro de control → Duplicar pantalla*. Reflex hace lo mismo, sin instalar nada en el iPhone.

Limitación de Apple: **no se puede controlar el iPhone desde el PC** (ni con AirPlay ni con ninguna API pública). Solo se ve y se oye.

## Cómo probarlo

1. En la pantalla de inicio, **iPhone / iPad**: el receptor AirPlay se activa al abrirla. Si Windows pregunta, permite el acceso a la red (al menos en redes privadas).
2. En el iPhone, con la misma Wi-Fi: *Centro de control → Duplicar pantalla →* «Reflex».
3. Para terminar: *Duplicar pantalla → Detener* en el iPhone, o «Detener receptor» en Reflex.

Si algo falla, *Ajustes → Registro* abre la carpeta con el registro (`%LOCALAPPDATA%\Reflex\logs\reflex.log`), donde queda cada paso del protocolo: conexión, emparejado, FairPlay, vídeo y audio.

## Estado

| # | Pieza | Estado | Código |
|---|---|---|---|
| 1 | Anuncio mDNS (`_airplay._tcp` y `_raop._tcp`, perfil de Apple TV 3) | ✅ | `src/iphone/mdns.rs` |
| 2 | Servidor RTSP: `GET /info`, `/pair-setup`, `/pair-verify`, `/fp-setup`, `SETUP`, `RECORD`, `GET_PARAMETER`, `SET_PARAMETER`, `TEARDOWN`, `/feedback` | ✅ | `src/iphone/rtsp.rs` |
| 3 | Emparejado legacy: Ed25519 + X25519, firma cifrada con AES-128-CTR | ✅ | `src/iphone/pairing.rs` |
| 4 | FairPlay SAP v3 (`fp-setup` y descifrado de `ekey`) | ✅ port a Rust de *playfair* | `src/iphone/fairplay/` |
| 5 | Vídeo H.264: cabecera de 128 bytes, AVCC, AES-128-CTR, decodificación con OpenH264 | ✅ | `src/iphone/mirror.rs` |
| 6 | Audio: RTP/UDP, AES-128-CBC, AAC-ELD, volumen del iPhone | ✅ decodificador AAC-ELD propio en Rust | `src/iphone/audio.rs`, `src/iphone/aac/` |
| 7 | Reloj NTP (el receptor pide la hora al iPhone) | ✅ básico, sin alinear audio y vídeo | `src/iphone/ntp.rs` |
| 8 | Interfaz: tarjeta en Inicio, pantalla del receptor, espejo, captura, grabación, pantalla completa | ✅ | `src/app.rs` |
| 9 | FairPlay v2 (iOS lo pide en algunos casos) | ⏳ se registra en el log si llega | |
| 10 | Grabación con sonido, H.265, PIN, varios iPhone a la vez | ⏳ (la grabación MP4 es solo de vídeo) | |

Todo lo anterior está cubierto por pruebas (`cargo test`), incluida una sesión completa contra un iPhone simulado: `/info`, emparejado, FairPlay con vectores reales, los dos `SETUP`, NTP, vídeo cifrado, volumen y `TEARDOWN`. El vídeo ya se ha probado con un iPhone 15 Pro Max; el audio, de momento, solo con las pruebas.

### Calidad

El iPhone ajusta su imagen a la **altura** de la pantalla que anuncia el receptor (el ancho sale de la forma de su pantalla). Con 1080 un iPhone en vertical queda en unos 500×1080; con 2160, en unos 1000×2160. Reflex pide siempre 2160: más resolución (la nativa de un iPhone 15 Pro Max es 1290×2796) costaría casi el doble de decodificar en CPU y no llegaría a 60 fps.

A esa resolución cada fotograma pesa 8,6 MB en RGBA. La conversión de color de OpenH264 solo usa SIMD con anchos múltiplos de 8 y el iPhone manda 998, así que Reflex convierte con su propia rutina entera repartida entre núcleos, y no convierte los fotogramas que la interfaz no llegaría a pintar. Antes, decodificar y convertir costaba 26 ms por fotograma; ahora, unos 15.

El SPS del iPhone se pasa al decodificador tal cual. Una versión anterior subía `max_num_ref_frames` de 4 a 16 para dar más memoria de imágenes a OpenH264, y estropeaba la imagen: el codificador del iPhone usa referencias a largo plazo, que van detrás de las de corto plazo en la lista de referencias, y con más imágenes de corto plazo guardadas un mismo índice apuntaba a otra imagen (lo estático salía bien y lo que se movía, desplazado). Si OpenH264 se queda sin memoria de imágenes, el decodificador se reinicia.

El crate `openh264` crea el decodificador con la ocultación de errores desactivada: tras perder una sola referencia no vuelve a dar imagen hasta el siguiente fotograma clave, y el iPhone casi nunca los manda (la pantalla se quedaba congelada mientras el audio seguía). `src/h264.rs` crea el decodificador directamente con `openh264-sys2` y la ocultación activada. Además, la configuración (SPS/PPS) solo se vuelve a dar al decodificador si cambia o con un fotograma clave: el iPhone la reenvía tras cada pausa y OpenH264 trata un SPS repetido como una secuencia nueva, descartando las referencias.

### FairPlay

No hay documentación oficial. `src/iphone/fairplay/` es una traducción sentencia a sentencia de *playfair* (EstebanKubata, GPL-3.0), la implementación por ingeniería inversa que usan shairplay, RPiPlay y UxPlay. Las tablas (`tables.rs`) se generan a partir de sus fuentes en C.

El port se comprobó función a función contra el C original compilado aparte, con 4000 vectores aleatorios (`garble`, `sap_hash`, MD5 modificado, key schedule, `cycle`, descifrado del mensaje, clave de sesión y `playfair_decrypt` completo). En el repositorio quedan cuatro vectores de extremo a extremo, uno por modo. La prueba exhaustiva sigue disponible:

```sh
REFLEX_FAIRPLAY_VECTORS=vectores.txt cargo test matches_c_reference -- --ignored
```

Cada línea del fichero es `<función> <entradas en hex separadas por comas> <salidas>`.

### AAC-ELD

Al duplicar, el iPhone manda el sonido en AAC-ELD (44,1 kHz, estéreo, tramas de 480, sin SBR; AudioSpecificConfig `f8e85000`). No hay decodificador de ELD en Rust, así que `src/iphone/aac/` implementa ese perfil: lectura de la trama ER, Huffman, factores de escala, TNS, M/S, estéreo de intensidad, PNS, decuantización y el banco de filtros de bajo retardo. Es un port del decodificador AAC de FFmpeg (LGPL-2.1+); `tables.rs` se genera a partir de su `aactab.c`.

Se comprueba contra fdk-aac, que se usa solo como oráculo en las pruebas (no forma parte de Reflex): se codificó audio de prueba (tonos, ruido, música y transitorios) en AAC-ELD a 24, 48, 96, 160 y 256 kbps y se comparó con lo que decodifica fdk. Coincide con un error máximo de 2–3 LSB de 16 bits (unos 85 dB de SNR). Las tramas con PNS no se pueden comparar muestra a muestra (cada decodificador genera su propio ruido); en ellas se compara la energía en agudos, que coincide en ±0,01 dB. En el repositorio quedan 12 tramas y su PCM de referencia (`src/iphone/aac/testdata/`); la comparación completa sigue disponible:

```sh
REFLEX_AAC_VECTORS=<carpeta con eld_<kbps>.bin y .pcm> cargo test --release matches_fdk_reference -- --ignored
```

## Requisitos de red

- PC e iPhone en la misma red Wi-Fi, sin aislamiento de clientes (algunas redes de invitados lo activan).
- El cortafuegos de Windows debe dejar a Reflex aceptar conexiones TCP (puerto 7000 y los de datos, que se eligen al vuelo) y UDP (5353 para mDNS, más los puertos de audio y del reloj). Windows lo pregunta la primera vez que se activa el receptor.
- Solo se anuncia por IPv4.

## Referencias

- UxPlay (GPL-3.0): https://github.com/FDH2/UxPlay
- playfair (GPL-3.0): https://github.com/EstebanKubata/playfair
- FFmpeg, decodificador AAC (LGPL-2.1+): https://github.com/FFmpeg/FFmpeg/tree/master/libavcodec/aac
- RPiPlay (GPL-3.0): https://github.com/FD-/RPiPlay
- Especificación no oficial de AirPlay: https://openairplay.github.io/airplay-spec/
