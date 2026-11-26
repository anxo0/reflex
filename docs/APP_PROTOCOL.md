# Reflex app protocol (Android app to PC)

The Reflex app for Android sends its screen (and sound) to a PC running Reflex. On the PC it is received in **From the phone**, together with Miracast.

## Discovery

- While **From the phone** is open, the PC announces `_reflex._tcp.local.` over mDNS. Instance name: the PC name. TXT: `name=<PC name>`, `v=1`.
- The PC also shows a QR code with `reflex://<ip>:<port>?c=<code>&n=<PC name>` and a 4-digit code that changes every time the screen is opened.
- TCP port 27184, or a random one if it is taken (the phone reads it from mDNS or from the QR).

## Framing

Every message, in both directions, is `[type u8][length u32 big-endian][payload]`.

### Phone to PC

| Type | Name | Payload |
|---|---|---|
| `0x01` | HELLO | UTF-8 `key=value` lines: `v=1`, `name` (phone model), `id` (stable random id of the install), `code` (4 digits, empty if the PC was picked from the list), `w`, `h` (video size), `audio=1/0`, `control=1/0` |
| `0x02` | VIDEO_CONFIG | H.264 SPS and PPS, Annex B |
| `0x03` | VIDEO | `pts u64` (microseconds) + `flags u8` (bit 0: key frame) + H.264 Annex B |
| `0x04` | AUDIO | PCM 16-bit little-endian, stereo, 48 kHz |
| `0x05` | SIZE | `width u32` + `height u32` (the screen rotated) |
| `0x06` | CONTROL | `u8`: 1 if the accessibility service is on and the PC may control the phone |
| `0x07` | BYE | empty |

### PC to phone

| Type | Name | Payload |
|---|---|---|
| `0x81` | ACCEPT | `u8` 1 accepted, 0 rejected, then a UTF-8 reason |
| `0x82` | TOUCH | `action u8` (0 down, 1 move, 2 up) + `x f32` + `y f32` (big-endian, 0..1 of the video) |
| `0x83` | KEY | `u8`: 1 back, 2 home, 3 recent apps, 4 notifications, 5 lock screen |
| `0x84` | BYE | empty |

## Acceptance

1. A HELLO with the right code is accepted, and the phone `id` is remembered.
2. A HELLO from a remembered `id` is accepted.
3. Otherwise the PC asks "Let «Pixel 8» send its screen?". With no answer in 60 s it is rejected.

Only one phone at a time. Sending BYE or closing the socket ends the session.
