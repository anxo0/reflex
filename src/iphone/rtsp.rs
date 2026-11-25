//! Servidor RTSP de AirPlay (puerto 7000).
//!
//! Orden típico de un duplicado desde el iPhone:
//! `GET /info` → `POST /pair-setup` → `POST /pair-verify` (×2) → `POST /fp-setup` (×2)
//! → `SETUP` (claves y reloj) → `GET_PARAMETER`/`RECORD` → `SETUP` (flujo 110 de vídeo
//! y 96 de audio) → `POST /feedback` cada 2 s → `TEARDOWN`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use eframe::egui;
use plist::{Dictionary, Value};
use sha2::{Digest, Sha512};

use super::identity::Identity;
use super::{State, audio, fairplay, mdns, mirror, ntp, pairing};
use crate::android::session::{Shared, finish_recording};
use crate::logs::log;

/// Lo que comparten todas las conexiones del receptor.
pub struct Context {
    pub identity: Identity,
    pub name: String,
    /// Altura de la pantalla que anunciamos. El iPhone ajusta su imagen a esta
    /// altura (el ancho sale de la forma de su pantalla), así que manda la calidad.
    pub display_height: Arc<AtomicU32>,
    pub shared: Arc<Shared>,
    pub state: Arc<Mutex<State>>,
    pub stop: Arc<AtomicBool>,
    pub egui: egui::Context,
    /// Aviso de cancelación del flujo de vídeo activo, para cortarlo en TEARDOWN o al parar.
    pub mirror: Mutex<Option<Arc<AtomicBool>>>,
    /// Ganancia del audio (bits de un `f32`), según el volumen del iPhone.
    pub volume: Arc<AtomicU32>,
    /// Cada flujo de vídeo nuevo incrementa la generación; así un hilo viejo que
    /// termina tarde no pisa el estado del nuevo.
    pub generation: AtomicU64,
}

impl Context {
    pub fn set_state(&self, s: State) {
        *self.state.lock().unwrap() = s;
        self.egui.request_repaint();
    }

    pub fn stop_mirror(&self) {
        if let Some(cancel) = self.mirror.lock().unwrap().take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

struct Request {
    method: String,
    url: String,
    protocol: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }
}

struct Response {
    status: u16,
    reason: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    close: bool,
}

impl Response {
    fn ok() -> Self {
        Self { status: 200, reason: "OK", headers: Vec::new(), body: Vec::new(), close: false }
    }

    fn error(status: u16, reason: &'static str) -> Self {
        Self { status, reason, ..Self::ok() }
    }

    fn with_body(mut self, content_type: &str, body: Vec<u8>) -> Self {
        self.headers.push(("Content-Type", content_type.into()));
        self.body = body;
        self
    }

    fn plist(self, value: Value) -> Self {
        let mut out = Vec::new();
        value.to_writer_binary(&mut out).expect("serializar plist en memoria no falla");
        self.with_body("application/x-apple-binary-plist", out)
    }
}

/// Estado de una conexión RTSP.
struct Conn {
    ctx: Arc<Context>,
    peer: SocketAddr,
    local: IpAddr,
    verify: Option<pairing::Verify>,
    keymsg: Option<[u8; 164]>,
    aes_key: Option<[u8; 16]>,
    aes_iv: Option<[u8; 16]>,
    timing_stop: Arc<AtomicBool>,
    /// Puertos del flujo de audio y aviso para parar su hilo.
    audio: Vec<UdpSocket>,
    audio_cancel: Option<Arc<AtomicBool>>,
    owns_mirror: Option<u64>,
}

pub fn serve(listener: TcpListener, ctx: Arc<Context>) {
    listener.set_nonblocking(true).ok();
    while !ctx.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, peer)) => {
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle(stream, peer, ctx) {
                        log(format!("AirPlay: conexión con {peer} terminada: {e:#}"));
                    }
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                log(format!("AirPlay: error aceptando conexiones: {e}"));
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

fn handle(stream: TcpStream, peer: SocketAddr, ctx: Arc<Context>) -> Result<()> {
    // No bloqueante, con esperas cortas: en Windows, tras un timeout de lectura
    // (SO_RCVTIMEO) el estado del socket queda indeterminado.
    stream.set_nonblocking(true)?;
    stream.set_nodelay(true).ok();
    let local = stream.local_addr()?.ip();
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut conn = Conn {
        ctx,
        peer,
        local,
        verify: None,
        keymsg: None,
        aes_key: None,
        aes_iv: None,
        timing_stop: Arc::new(AtomicBool::new(false)),
        audio: Vec::new(),
        audio_cancel: None,
        owns_mirror: None,
    };
    log(format!("AirPlay: conexión de {peer}"));

    let result = (|| -> Result<()> {
        while !conn.ctx.stop.load(Ordering::Relaxed) {
            let Some(req) = read_request(&mut reader, &conn.ctx.stop)? else { break };
            let res = conn.dispatch(&req);
            write_response(&mut writer, &req, &res)?;
            if res.close {
                break;
            }
        }
        Ok(())
    })();
    conn.end_session();
    result
}

fn read_request(r: &mut BufReader<TcpStream>, stop: &AtomicBool) -> Result<Option<Request>> {
    let mut line = String::new();
    // Espera a la siguiente petición sin bloquear el cierre del receptor.
    loop {
        match r.read_line(&mut line) {
            Ok(0) => return Ok(None),
            Ok(_) if line.trim().is_empty() => line.clear(),
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if stop.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(e.into()),
        }
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(url), Some(protocol)) = (parts.next(), parts.next(), parts.next()) else {
        bail!("línea de petición no válida: {:?}", line.trim());
    };
    let (method, url, protocol) = (method.to_string(), url.to_string(), protocol.to_string());

    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        read_line_blocking(r, &mut h)?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
        if headers.len() > 64 {
            bail!("demasiadas cabeceras");
        }
    }
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if len > 1024 * 1024 {
        bail!("cuerpo demasiado grande ({len} bytes)");
    }
    let mut body = vec![0u8; len];
    let mut read = 0;
    while read < len {
        match r.read(&mut body[read..]) {
            Ok(0) => bail!("conexión cerrada a mitad de la petición"),
            Ok(n) => read += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Some(Request { method, url, protocol, headers, body }))
}

fn read_line_blocking(r: &mut BufReader<TcpStream>, out: &mut String) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match r.read_line(out) {
            Ok(0) => bail!("conexión cerrada a mitad de la petición"),
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    bail!("petición incompleta");
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn write_response(w: &mut TcpStream, req: &Request, res: &Response) -> Result<()> {
    let mut head = format!("{} {} {}\r\n", req.protocol, res.status, res.reason);
    let cseq = req.header("CSeq");
    if let Some(c) = cseq {
        head.push_str(&format!("CSeq: {c}\r\n"));
        if req.method != "RECORD" {
            head.push_str("Audio-Jack-Status: connected; type=digital\r\n");
        }
    }
    head.push_str(&format!("Server: AirTunes/{}\r\n", mdns::SOURCE_VERSION));
    for (k, v) in &res.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if res.close {
        head.push_str("Connection: close\r\n");
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", res.body.len()));
    write_all_nonblocking(w, head.as_bytes())?;
    write_all_nonblocking(w, &res.body)?;
    Ok(())
}

/// `write_all` para un socket no bloqueante: espera si el búfer de envío está lleno.
fn write_all_nonblocking(w: &mut TcpStream, mut data: &[u8]) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !data.is_empty() {
        match w.write(data) {
            Ok(0) => bail!("el iPhone cerró la conexión"),
            Ok(n) => data = &data[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

impl Conn {
    fn dispatch(&mut self, req: &Request) -> Response {
        let r = match (req.method.as_str(), req.url.as_str()) {
            ("GET", u) if u.starts_with("/info") => self.info(req),
            ("POST", "/pair-setup") => self.pair_setup(req),
            ("POST", "/pair-verify") => self.pair_verify(req),
            ("POST", "/fp-setup") => self.fp_setup(req),
            ("POST", "/feedback" | "/audioMode") => Ok(Response::ok()),
            ("OPTIONS", _) => {
                let mut r = Response::ok();
                r.headers.push(("Public", "SETUP, RECORD, FLUSH, TEARDOWN, OPTIONS, GET_PARAMETER, SET_PARAMETER".into()));
                Ok(r)
            }
            ("SETUP", _) => self.setup(req),
            ("GET_PARAMETER", _) => Ok(get_parameter(req)),
            ("SET_PARAMETER", _) => Ok(self.set_parameter(req)),
            ("FLUSH", _) => Ok(Response::ok()),
            ("RECORD", _) => {
                let mut r = Response::ok();
                r.headers.push(("Audio-Latency", "0".into()));
                r.headers.push(("Audio-Jack-Status", "connected; type=analog".into()));
                Ok(r)
            }
            ("TEARDOWN", _) => self.teardown(req),
            (m, u) => {
                log(format!("AirPlay: petición no soportada {m} {u}"));
                Ok(Response::ok())
            }
        };
        r.unwrap_or_else(|e| {
            log(format!("AirPlay: {} {} falló: {e:#}", req.method, req.url));
            Response::error(400, "Bad Request")
        })
    }

    fn info(&self, req: &Request) -> Result<Response> {
        let id = &self.ctx.identity;
        let txt = Value::Data(mdns::encode_txt(&mdns::airplay_txt(id)));

        // Primera consulta: un plist que pide solo el TXT de `_airplay._tcp`.
        let is_plist = req.header("Content-Type").is_some_and(|c| c.contains("apple-binary-plist"));
        if is_plist {
            let wants_txt = Value::from_reader(Cursor::new(&req.body))
                .ok()
                .and_then(|v| v.as_dictionary()?.get("qualifier")?.as_array()?.first()?.as_string().map(|s| s == "txtAirPlay"))
                .unwrap_or(false);
            let mut d = Dictionary::new();
            if wants_txt {
                d.insert("txtAirPlay".into(), txt);
            }
            return Ok(Response::ok().plist(Value::Dictionary(d)));
        }

        let uint = |v: u64| Value::Integer(v.into());
        let mut d = Dictionary::new();
        if req.header("CSeq").is_none() && req.url.contains("txtAirPlay") {
            d.insert("txtAirPlay".into(), txt);
        }
        d.insert("deviceID".into(), Value::String(id.device_id()));
        d.insert("macAddress".into(), Value::String(id.device_id()));
        d.insert("pk".into(), Value::Data(id.public_key().to_vec()));
        d.insert("features".into(), uint(mdns::FEATURES));
        d.insert("name".into(), Value::String(self.ctx.name.clone()));
        d.insert("pi".into(), Value::String(mdns::PI.into()));
        d.insert("vv".into(), uint(2));
        d.insert("statusFlags".into(), uint(68));
        d.insert("keepAliveLowPower".into(), Value::Boolean(true));
        d.insert("sourceVersion".into(), Value::String(mdns::SOURCE_VERSION.into()));
        d.insert("keepAliveSendStatsAsBody".into(), Value::Boolean(true));
        d.insert("model".into(), Value::String(mdns::MODEL.into()));
        if req.header("CSeq").is_some() {
            d.insert("initialVolume".into(), Value::Real(0.0));
            let audio = |ty: u64, key: &str, val: Value| {
                let mut a = Dictionary::new();
                a.insert("type".into(), uint(ty));
                a.insert(key.into(), val);
                a
            };
            let latencies = [100, 101].map(|ty| {
                let mut a = audio(ty, "audioType", Value::String("default".into()));
                a.insert("inputLatencyMicros".into(), uint(0));
                a.insert("outputLatencyMicros".into(), Value::Boolean(false));
                Value::Dictionary(a)
            });
            d.insert("audioLatencies".into(), Value::Array(latencies.to_vec()));
            let formats = [100, 101].map(|ty| {
                let mut a = audio(ty, "audioInputFormats", uint(0x3ff_fffc));
                a.insert("audioOutputFormats".into(), uint(0x3ff_fffc));
                Value::Dictionary(a)
            });
            d.insert("audioFormats".into(), Value::Array(formats.to_vec()));

            let mut display = Dictionary::new();
            display.insert("uuid".into(), Value::String("e0ff8a27-6738-3d56-8a16-cc53aacee925".into()));
            display.insert("widthPhysical".into(), uint(0));
            display.insert("heightPhysical".into(), uint(0));
            let height = u64::from(self.ctx.display_height.load(Ordering::Relaxed));
            let width = height * 16 / 9;
            display.insert("width".into(), uint(width));
            display.insert("height".into(), uint(height));
            display.insert("widthPixels".into(), uint(width));
            display.insert("heightPixels".into(), uint(height));
            display.insert("rotation".into(), Value::Boolean(false));
            display.insert("refreshRate".into(), Value::Real(1.0 / 60.0));
            display.insert("maxFPS".into(), uint(60));
            display.insert("overscanned".into(), Value::Boolean(false));
            display.insert("features".into(), uint(14));
            d.insert("displays".into(), Value::Array(vec![Value::Dictionary(display)]));
        }
        Ok(Response::ok().plist(Value::Dictionary(d)))
    }

    fn pair_setup(&self, req: &Request) -> Result<Response> {
        if req.body.len() != 32 {
            bail!("pair-setup de {} bytes (se esperaban 32)", req.body.len());
        }
        Ok(Response::ok().with_body("application/octet-stream", self.ctx.identity.public_key().to_vec()))
    }

    fn pair_verify(&mut self, req: &Request) -> Result<Response> {
        match req.body.first() {
            Some(1) => {
                let (state, body) = pairing::verify_start(&self.ctx.identity, &req.body)?;
                self.verify = Some(state);
                Ok(Response::ok().with_body("application/octet-stream", body))
            }
            Some(0) => {
                let state = self.verify.as_mut().context("pair-verify sin primer paso")?;
                if let Err(e) = pairing::verify_finish(state, &req.body) {
                    log(format!("AirPlay: emparejado rechazado: {e:#}"));
                    let mut r = Response::error(470, "Connection Authorization Required");
                    r.close = true;
                    return Ok(r);
                }
                log("AirPlay: emparejado verificado");
                Ok(Response::ok().with_body("application/octet-stream", Vec::new()))
            }
            _ => bail!("pair-verify con cabecera desconocida"),
        }
    }

    fn fp_setup(&mut self, req: &Request) -> Result<Response> {
        if req.body.len() >= 5 && req.body[4] != 3 {
            log(format!(
                "AirPlay: el iPhone pide FairPlay v{}, y Reflex solo implementa la v3. Abre un issue con este registro.",
                req.body[4]
            ));
            return Ok(Response::error(501, "Not Implemented"));
        }
        let body = match req.body.len() {
            16 => fairplay::setup_reply(&req.body)?.to_vec(),
            164 => {
                let (res, keymsg) = fairplay::handshake_reply(&req.body)?;
                self.keymsg = Some(keymsg);
                res.to_vec()
            }
            n => bail!("fp-setup de {n} bytes"),
        };
        Ok(Response::ok().with_body("application/octet-stream", body))
    }

    fn setup(&mut self, req: &Request) -> Result<Response> {
        let root = Value::from_reader(Cursor::new(&req.body)).context("SETUP sin plist válido")?;
        let root = root.as_dictionary().context("SETUP: el plist no es un diccionario")?;
        let mut res = Dictionary::new();

        if let (Some(ekey), Some(eiv)) = (root.get("ekey").and_then(Value::as_data), root.get("eiv").and_then(Value::as_data)) {
            self.aes_iv = eiv.try_into().ok();
            let keymsg = self.keymsg.as_ref().context("SETUP antes de fp-setup")?;
            let mut key = fairplay::decrypt_key(keymsg, ekey)?;
            // Con emparejado, la clave se mezcla con el secreto ECDH.
            if let Some(v) = &self.verify {
                let h = Sha512::new().chain_update(key).chain_update(v.shared_secret).finalize();
                key.copy_from_slice(&h[..16]);
            }
            self.aes_key = Some(key);

            let name = root.get("name").and_then(Value::as_string).unwrap_or("iPhone");
            let model = root.get("model").and_then(Value::as_string).unwrap_or("?");
            log(format!("AirPlay: {name} ({model}) quiere duplicar la pantalla"));
            *self.ctx.shared.device_name.lock().unwrap() = name.to_string();

            if let Some(port) = root.get("timingPort").and_then(uint) {
                self.timing_stop.store(true, Ordering::Relaxed);
                self.timing_stop = Arc::new(AtomicBool::new(false));
                let local = ntp::start(SocketAddr::new(self.peer.ip(), port as u16), self.timing_stop.clone())?;
                res.insert("timingPort".into(), Value::Integer(u64::from(local).into()));
            }
            res.insert("eventPort".into(), Value::Integer(0u64.into()));
        }

        if let Some(streams) = root.get("streams").and_then(Value::as_array) {
            let mut out = Vec::new();
            for s in streams {
                let s = s.as_dictionary().context("stream no válido")?;
                match s.get("type").and_then(uint) {
                    Some(110) => {
                        let id = s.get("streamConnectionID").and_then(uint).context("falta streamConnectionID")?;
                        let port = self.start_mirror(id)?;
                        let mut d = Dictionary::new();
                        d.insert("type".into(), Value::Integer(110u64.into()));
                        d.insert("dataPort".into(), Value::Integer(u64::from(port).into()));
                        out.push(Value::Dictionary(d));
                    }
                    Some(96) => {
                        let data = UdpSocket::bind((self.local, 0))?;
                        let control = UdpSocket::bind((self.local, 0))?;
                        let mut d = Dictionary::new();
                        d.insert("type".into(), Value::Integer(96u64.into()));
                        d.insert("dataPort".into(), Value::Integer(u64::from(data.local_addr()?.port()).into()));
                        d.insert("controlPort".into(), Value::Integer(u64::from(control.local_addr()?.port()).into()));
                        self.start_audio(&data, s.get("ct").and_then(uint));
                        self.audio = vec![data, control];
                        out.push(Value::Dictionary(d));
                    }
                    other => log(format!("AirPlay: flujo de tipo {other:?} no soportado")),
                }
            }
            res.insert("streams".into(), Value::Array(out));
        }
        Ok(Response::ok().plist(Value::Dictionary(res)))
    }

    fn start_mirror(&mut self, stream_id: u64) -> Result<u16> {
        let key = self.aes_key.context("flujo de vídeo sin clave: falta el primer SETUP")?;
        let ctx = self.ctx.clone();
        ctx.stop_mirror();
        let generation = ctx.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.owns_mirror = Some(generation);

        let cancel = Arc::new(AtomicBool::new(false));
        *ctx.mirror.lock().unwrap() = Some(cancel.clone());

        let listener = TcpListener::bind((self.local, 0)).context("no se pudo abrir el puerto de vídeo")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            let stream = loop {
                if ctx.stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                    return;
                }
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        log(format!("AirPlay: el iPhone no abrió el flujo de vídeo: {e}"));
                        return;
                    }
                }
            };
            drop(listener);
            if stream.set_nonblocking(false).is_err() {
                return;
            }
            let name = ctx.shared.device_name.lock().unwrap().clone();
            ctx.set_state(State::Mirroring(name));
            log("AirPlay: recibiendo vídeo");

            let r = mirror::run(stream, mirror::stream_cipher(&key, stream_id), &ctx.shared, &ctx.egui, &cancel);
            if ctx.generation.load(Ordering::SeqCst) != generation {
                return; // ya hay otro flujo en marcha
            }
            match r {
                Ok(()) => log("AirPlay: el iPhone dejó de duplicar"),
                Err(e) => {
                    log(format!("AirPlay: el vídeo se cortó: {e:#}"));
                    ctx.shared.notice(format!("iPhone: {e:#}"));
                }
            }
            finish_recording(&ctx.shared);
            reset_frame(&ctx.shared);
            if !ctx.stop.load(Ordering::Relaxed) {
                ctx.set_state(State::Waiting);
            }
        });
        Ok(port)
    }

    fn start_audio(&mut self, data: &UdpSocket, ct: Option<u64>) {
        self.stop_audio();
        if let Err(e) = audio::check_codec(ct) {
            log(format!("AirPlay: sin audio: {e:#}"));
            self.ctx.shared.notice(format!("iPhone: {e:#}"));
            return;
        }
        let (Some(key), Some(iv)) = (self.aes_key, self.aes_iv) else {
            log("AirPlay: flujo de audio sin clave: falta el primer SETUP");
            return;
        };
        let sock = match data.try_clone() {
            Ok(s) => s,
            Err(e) => {
                log(format!("AirPlay: no se pudo preparar el audio: {e}"));
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.audio_cancel = Some(cancel.clone());
        let stream = audio::Stream { key, iv, gain: self.ctx.volume.clone(), cancel };
        let shared = self.ctx.shared.clone();
        std::thread::spawn(move || {
            if let Err(e) = audio::run(sock, stream) {
                log(format!("AirPlay: el audio se cortó: {e:#}"));
                shared.notice(format!("Audio del iPhone: {e:#}"));
            }
        });
    }

    fn stop_audio(&mut self) {
        if let Some(c) = self.audio_cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.audio.clear();
    }

    /// `volume: <dB>` ajusta el volumen; el resto (metadatos, progreso) se ignora.
    fn set_parameter(&self, req: &Request) -> Response {
        if let Some(v) = std::str::from_utf8(&req.body).ok().and_then(|b| b.trim().strip_prefix("volume:")).and_then(|v| v.trim().parse::<f32>().ok()) {
            self.ctx.volume.store(audio::gain_from_volume(v).to_bits(), Ordering::Relaxed);
        }
        Response::ok()
    }

    fn teardown(&mut self, req: &Request) -> Result<Response> {
        let types: Vec<u64> = Value::from_reader(Cursor::new(&req.body))
            .ok()
            .and_then(|v| {
                let streams = v.as_dictionary()?.get("streams")?.as_array()?.clone();
                Some(streams.iter().filter_map(|s| s.as_dictionary()?.get("type").and_then(uint)).collect())
            })
            .unwrap_or_default();
        if types.is_empty() || types.contains(&96) {
            self.stop_audio();
        }
        // Sin lista de flujos (o con el de vídeo) se termina todo: iOS reciente no
        // siempre manda el TEARDOWN del 110 al dejar de duplicar.
        let mut r = Response::ok();
        if types.is_empty() || types.contains(&110) {
            self.stop_own_mirror();
        }
        if types.is_empty() {
            r.close = true;
        }
        Ok(r)
    }

    fn stop_own_mirror(&mut self) {
        if let Some(g) = self.owns_mirror.take()
            && self.ctx.generation.load(Ordering::SeqCst) == g
        {
            self.ctx.stop_mirror();
        }
    }

    fn end_session(&mut self) {
        self.timing_stop.store(true, Ordering::Relaxed);
        self.stop_audio();
        self.stop_own_mirror();
    }
}

fn get_parameter(req: &Request) -> Response {
    if req.body.starts_with(b"volume") {
        return Response::ok().with_body("text/parameters", b"volume: 0.000000\r\n".to_vec());
    }
    Response::ok()
}

fn uint(v: &Value) -> Option<u64> {
    v.as_unsigned_integer().or_else(|| v.as_signed_integer().map(|i| i as u64))
}

fn reset_frame(shared: &Shared) {
    let mut f = shared.frame.lock().unwrap();
    f.width = 0;
    f.height = 0;
    f.image = None;
    f.seq += 1;
    drop(f);
    *shared.video_size.lock().unwrap() = (0, 0);
    shared.shown_seq.store(u64::MAX, Ordering::Relaxed);
    *shared.fps.lock().unwrap() = 0.0;
}

/// Un "iPhone" de prueba recorre el protocolo completo contra el servidor real.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::iphone::fairplay::tests::{VECTORS, unhex};
    use crate::iphone::mirror::tests::{sample_stream, send_stream};
    use ctr::cipher::{KeyIvInit, StreamCipher};
    use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
    use x25519_dalek::{PublicKey, StaticSecret};

    struct Client {
        s: BufReader<TcpStream>,
        cseq: u32,
    }

    impl Client {
        fn call(&mut self, method: &str, url: &str, content_type: Option<&str>, body: &[u8]) -> (u16, HashMap<String, String>, Vec<u8>) {
            self.cseq += 1;
            let mut req = format!("{method} {url} RTSP/1.0\r\nCSeq: {}\r\nContent-Length: {}\r\n", self.cseq, body.len());
            if let Some(ct) = content_type {
                req.push_str(&format!("Content-Type: {ct}\r\n"));
            }
            req.push_str("\r\n");
            let w = self.s.get_mut();
            w.write_all(req.as_bytes()).unwrap();
            w.write_all(body).unwrap();

            let mut line = String::new();
            self.s.read_line(&mut line).unwrap();
            let status: u16 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
            let mut headers = HashMap::new();
            loop {
                let mut h = String::new();
                self.s.read_line(&mut h).unwrap();
                let h = h.trim_end();
                if h.is_empty() {
                    break;
                }
                let (k, v) = h.split_once(':').unwrap();
                headers.insert(k.to_ascii_lowercase(), v.trim().to_string());
            }
            assert_eq!(headers["cseq"], self.cseq.to_string());
            let len: usize = headers["content-length"].parse().unwrap();
            let mut body = vec![0; len];
            self.s.read_exact(&mut body).unwrap();
            (status, headers, body)
        }

        fn plist(&mut self, method: &str, url: &str, value: Value) -> Dictionary {
            let mut body = Vec::new();
            value.to_writer_binary(&mut body).unwrap();
            let (status, _, res) = self.call(method, url, Some("application/x-apple-binary-plist"), &body);
            assert_eq!(status, 200, "{method} {url}");
            Value::from_reader(Cursor::new(res)).unwrap().into_dictionary().unwrap()
        }
    }

    fn dict(items: Vec<(&str, Value)>) -> Value {
        Value::Dictionary(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn sha512_16(parts: &[&[u8]]) -> [u8; 16] {
        let mut h = Sha512::new();
        for p in parts {
            h.update(p);
        }
        h.finalize()[..16].try_into().unwrap()
    }

    #[test]
    fn full_mirroring_session() {
        let receiver_key = SigningKey::from_bytes(&[5; 32]);
        let receiver_pk = receiver_key.verifying_key().to_bytes();
        let ctx = Arc::new(Context {
            identity: Identity { signing: receiver_key, device_id: [0x02, 1, 2, 3, 4, 5] },
            name: "Reflex prueba".into(),
            display_height: Arc::new(AtomicU32::new(1440)),
            shared: Arc::new(Shared::new()),
            state: Arc::new(Mutex::new(State::Waiting)),
            stop: Arc::new(AtomicBool::new(false)),
            egui: egui::Context::default(),
            mirror: Mutex::new(None),
            volume: Arc::new(AtomicU32::new(1f32.to_bits())),
            generation: AtomicU64::new(0),
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server_ctx = ctx.clone();
        std::thread::spawn(move || serve(listener, server_ctx));

        let s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut c = Client { s: BufReader::new(s), cseq: 0 };

        // GET /info: primero solo el TXT, luego la descripción completa.
        let info = c.plist("GET", "/info", dict(vec![("qualifier", Value::Array(vec![Value::String("txtAirPlay".into())]))]));
        let txt = info["txtAirPlay"].as_data().unwrap();
        assert!(txt.windows(16).any(|w| w == b"model=AppleTV3,2"));
        let (status, _, body) = c.call("GET", "/info", None, &[]);
        assert_eq!(status, 200);
        let info = Value::from_reader(Cursor::new(body)).unwrap().into_dictionary().unwrap();
        assert_eq!(info["pk"].as_data().unwrap(), receiver_pk);
        assert_eq!(info["features"].as_unsigned_integer(), Some(mdns::FEATURES));
        assert_eq!(info["name"].as_string(), Some("Reflex prueba"));
        let display = info["displays"].as_array().unwrap()[0].as_dictionary().unwrap();
        assert_eq!(display["height"].as_unsigned_integer(), Some(1440));
        assert_eq!(display["width"].as_unsigned_integer(), Some(2560));

        // Emparejado.
        let client_ed = SigningKey::from_bytes(&[6; 32]);
        let (status, _, body) = c.call("POST", "/pair-setup", Some("application/octet-stream"), &client_ed.verifying_key().to_bytes());
        assert_eq!((status, body.as_slice()), (200, receiver_pk.as_slice()));
        let client_x = StaticSecret::from([7; 32]);
        let client_pub = PublicKey::from(&client_x).to_bytes();
        let mut m1 = vec![1, 0, 0, 0];
        m1.extend_from_slice(&client_pub);
        m1.extend_from_slice(&client_ed.verifying_key().to_bytes());
        let (status, _, body) = c.call("POST", "/pair-verify", Some("application/octet-stream"), &m1);
        assert_eq!((status, body.len()), (200, 96));
        let server_pub: [u8; 32] = body[..32].try_into().unwrap();
        let shared_secret = client_x.diffie_hellman(&PublicKey::from(server_pub)).to_bytes();
        let key = sha512_16(&[b"Pair-Verify-AES-Key", &shared_secret]);
        let iv = sha512_16(&[b"Pair-Verify-AES-IV", &shared_secret]);
        let mut aes = ctr::Ctr128BE::<aes::Aes128>::new(&key.into(), &iv.into());
        let mut sig: [u8; 64] = body[32..].try_into().unwrap();
        aes.apply_keystream(&mut sig);
        let signed = [server_pub, client_pub].concat();
        VerifyingKey::from_bytes(&receiver_pk).unwrap().verify(&signed, &Signature::from_bytes(&sig)).unwrap();
        let mut our_sig = client_ed.sign(&[client_pub, server_pub].concat()).to_bytes();
        aes.apply_keystream(&mut our_sig);
        let (status, _, _) = c.call("POST", "/pair-verify", Some("application/octet-stream"), &[&[0u8, 0, 0, 0][..], &our_sig].concat());
        assert_eq!(status, 200);

        // FairPlay, con un mensaje y una ekey de los vectores del C de referencia.
        let (msg, ekey, aes_key) = VECTORS[0];
        let mut fp1 = [0u8; 16];
        fp1[4] = 3;
        fp1[14] = unhex(msg)[12];
        let (status, _, body) = c.call("POST", "/fp-setup", Some("application/octet-stream"), &fp1);
        assert_eq!((status, body.len()), (200, 142));
        let (status, _, body) = c.call("POST", "/fp-setup", Some("application/octet-stream"), &unhex(msg));
        assert_eq!((status, body.len()), (200, 32));

        // Primer SETUP: claves y reloj.
        let timing = UdpSocket::bind("127.0.0.1:0").unwrap();
        timing.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let res = c.plist(
            "SETUP",
            "rtsp://127.0.0.1/1",
            dict(vec![
                ("ekey", Value::Data(unhex(ekey))),
                ("eiv", Value::Data(vec![0; 16])),
                ("timingProtocol", Value::String("NTP".into())),
                ("timingPort", Value::Integer(u64::from(timing.local_addr().unwrap().port()).into())),
                ("name", Value::String("iPhone de prueba".into())),
                ("model", Value::String("iPhone15,2".into())),
            ]),
        );
        assert!(res["timingPort"].as_unsigned_integer().unwrap() > 0);
        let mut ntp_req = [0u8; 64];
        let (n, _) = timing.recv_from(&mut ntp_req).expect("el receptor debe pedir la hora");
        assert_eq!((n, &ntp_req[..2]), (32, &[0x80, 0xd2][..]));

        // Segundo SETUP: flujo de vídeo, con un streamConnectionID que llega como
        // entero de 64 bits con signo (negativo).
        let stream_id: i64 = -1_234_567_890_123;
        let video = dict(vec![("type", Value::Integer(110u64.into())), ("streamConnectionID", Value::Integer(stream_id.into()))]);
        let res = c.plist("SETUP", "rtsp://127.0.0.1/1", dict(vec![("streams", Value::Array(vec![video]))]));
        let stream = res["streams"].as_array().unwrap()[0].as_dictionary().unwrap();
        let data_port = stream["dataPort"].as_unsigned_integer().unwrap() as u16;
        let (status, _, _) = c.call("RECORD", "rtsp://127.0.0.1/1", None, &[]);
        assert_eq!(status, 200);

        // Vídeo cifrado con la clave que debe haber derivado el receptor.
        let fairplay_key = unhex(aes_key);
        let session_key = sha512_16(&[&fairplay_key, &shared_secret]);
        let (w, h) = (256, 192);
        let (avcc, frames) = sample_stream(w, h, 12);
        let n = frames.len() as u64;
        let mut data = TcpStream::connect(("127.0.0.1", data_port)).unwrap();
        send_stream(&mut data, mirror::stream_cipher(&session_key, stream_id as u64), avcc, frames, w, h);

        let deadline = Instant::now() + Duration::from_secs(10);
        while ctx.shared.frame.lock().unwrap().seq < n - 1 {
            assert!(Instant::now() < deadline, "no llegaron los fotogramas: {}", ctx.shared.frame.lock().unwrap().seq);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(*ctx.state.lock().unwrap(), State::Mirroring("iPhone de prueba".into()));
        assert_eq!(*ctx.shared.video_size.lock().unwrap(), (w as u32, h as u32));

        let (status, _, _) = c.call("POST", "/feedback", None, &[]);
        assert_eq!(status, 200);
        let (status, _, _) = c.call("SET_PARAMETER", "rtsp://127.0.0.1/1", Some("text/parameters"), b"volume: -20.000000
");
        assert_eq!(status, 200);
        assert!((f32::from_bits(ctx.volume.load(Ordering::Relaxed)) - 0.1).abs() < 1e-4);
        let teardown = dict(vec![("streams", Value::Array(vec![dict(vec![("type", Value::Integer(110u64.into()))])]))]);
        let (status, _, _) = {
            let mut body = Vec::new();
            teardown.to_writer_binary(&mut body).unwrap();
            c.call("TEARDOWN", "rtsp://127.0.0.1/1", Some("application/x-apple-binary-plist"), &body)
        };
        assert_eq!(status, 200);
        while *ctx.state.lock().unwrap() != State::Waiting {
            assert!(Instant::now() < deadline, "el receptor no volvió a la espera: {:?}", ctx.state.lock().unwrap());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(ctx.shared.frame.lock().unwrap().width, 0);
        ctx.stop.store(true, Ordering::Relaxed);
    }
}
