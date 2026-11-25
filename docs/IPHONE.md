# iPhone / iPad: hoja de ruta (fase 2)

## Por qué AirPlay

En Windows no hay forma pública de capturar la pantalla de un iPhone por cable (en macOS sí existe, vía QuickTime). Las apps de duplicado para iPhone usan **AirPlay**: el PC se anuncia en la red como un Apple TV y el iPhone le envía su pantalla desde *Centro de control → Duplicar pantalla*. Reflex hará lo mismo, sin instalar nada en el iPhone.

Limitación de Apple: **no se puede controlar el iPhone desde el PC** (ni con AirPlay ni con ninguna API pública). Solo se ve y se oye.

## Piezas a implementar

| # | Pieza | Detalle | Crates |
|---|---|---|---|
| 1 | Anuncio mDNS | Servicios `_airplay._tcp` y `_raop._tcp` con los TXT de un receptor (deviceid, features, model, pk…) | `mdns-sd` |
| 2 | Servidor RTSP/HTTP | Puerto 7000: `GET /info` (plist binario), `POST /pair-setup`, `/pair-verify`, `/fp-setup`, `SETUP`, `RECORD`, `GET_PARAMETER`, `SET_PARAMETER`, `TEARDOWN`, `/feedback` | `tokio`, `plist` |
| 3 | Emparejado | pair-setup / pair-verify con Ed25519 + X25519 y AES-CTR | `ed25519-dalek`, `x25519-dalek`, `aes`, `ctr`, `sha2` |
| 4 | FairPlay (`fp-setup`) | Handshake de FairPlay SAP en 2 fases para obtener la clave AES del flujo. No hay documentación oficial; hay que portar la implementación por ingeniería inversa de *playfair* (usada por shairplay/UxPlay, GPL). Por eso Reflex es GPL-3.0 | port a Rust |
| 5 | Flujo de vídeo | Puerto TCP de datos: cabecera de 128 bytes por paquete, carga H.264 (AVCC) cifrada con AES-128-CTR; clave = SHA-512(aeskey ‖ secreto ECDH) | reutiliza `video.rs` |
| 6 | Audio | AAC-ELD (duplicado) o ALAC por RTP/UDP, cifrado AES-CBC | `fdk-aac` o decodificador propio |
| 7 | Sincronía | NTP/PTP de AirPlay para alinear audio y vídeo | |
| 8 | Interfaz | Pestaña iPhone: estado del receptor, nombre visible, PIN opcional, misma vista de espejo, captura y grabación | `app.rs` |

## Orden previsto

1. mDNS + `/info`: el iPhone ya ve «Reflex» en la lista de Duplicar pantalla.
2. pair-setup/verify + FairPlay: el iPhone acepta la conexión.
3. Vídeo: descifrar y decodificar → primera imagen.
4. Audio y sincronía.
5. Pulido: PIN, reconexión, rotación, varios dispositivos.

## Requisitos de red

- PC e iPhone en la misma red Wi-Fi, sin aislamiento de clientes.
- El cortafuegos de Windows debe permitir a Reflex los puertos TCP 7000/7100 y UDP 5353 (mDNS) y los puertos RTP. La app lo pedirá la primera vez.

## Referencias

- UxPlay (GPL-3.0): https://github.com/FDH2/UxPlay
- Especificación no oficial de AirPlay: https://openairplay.github.io/airplay-spec/
- RPiPlay: https://github.com/FD-/RPiPlay
