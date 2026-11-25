//! Envoltorio mínimo sobre el ejecutable `adb`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context as _, Result, bail};

#[derive(Clone, Debug)]
pub struct Adb {
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub serial: String,
    /// `device`, `unauthorized`, `offline`…
    pub state: String,
    pub model: String,
}

impl Device {
    pub fn is_wireless(&self) -> bool {
        self.serial.contains(':') || self.serial.contains("._adb-tls-connect.")
    }
    pub fn label(&self) -> String {
        if self.model.is_empty() { self.serial.clone() } else { self.model.clone() }
    }
}

/// Evita que cada llamada a adb abra una ventana de consola en Windows.
pub fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

impl Adb {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn command(&self) -> Command {
        let mut c = Command::new(&self.path);
        hide_window(&mut c);
        c
    }

    fn output(&self, args: &[&str]) -> Result<Output> {
        self.command()
            .args(args)
            .output()
            .with_context(|| format!("no se pudo ejecutar adb {}", args.join(" ")))
    }

    pub fn run(&self, args: &[&str]) -> Result<String> {
        let out = self.output(args)?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            bail!("adb {} falló: {} {}", args.join(" "), stderr.trim(), stdout.trim());
        }
        Ok(stdout)
    }

    pub fn run_serial(&self, serial: &str, args: &[&str]) -> Result<String> {
        let mut full = vec!["-s", serial];
        full.extend_from_slice(args);
        self.run(&full)
    }

    pub fn start_server(&self) -> Result<()> {
        self.run(&["start-server"]).map(|_| ())
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        let out = self.run(&["devices", "-l"])?;
        Ok(parse_devices(&out))
    }

    pub fn push(&self, serial: &str, local: &Path, remote: &str) -> Result<()> {
        let local = local.to_string_lossy();
        self.run_serial(serial, &["push", &local, remote]).map(|_| ())
    }

    /// `adb forward tcp:0 <remote>` reserva un puerto libre y lo imprime.
    pub fn forward_auto(&self, serial: &str, remote: &str) -> Result<u16> {
        let out = self.run_serial(serial, &["forward", "tcp:0", remote])?;
        out.trim().parse().with_context(|| format!("respuesta inesperada de adb forward: {out:?}"))
    }

    pub fn forward_remove(&self, serial: &str, port: u16) {
        let _ = self.run_serial(serial, &["forward", "--remove", &format!("tcp:{port}")]);
    }

    pub fn shell(&self, serial: &str, cmd: &str) -> Result<String> {
        self.run_serial(serial, &["shell", cmd])
    }

    /// IP del Wi-Fi del móvil, para pasar de cable a inalámbrico.
    pub fn wifi_ip(&self, serial: &str) -> Option<String> {
        if let Ok(out) = self.shell(serial, "ip -4 -o addr show wlan0")
            && let Some(ip) = parse_inet(&out) {
                return Some(ip);
            }
        let out = self.shell(serial, "ip route").ok()?;
        out.lines()
            .filter(|l| l.contains("wlan"))
            .find_map(|l| {
                let mut it = l.split_whitespace();
                while let Some(tok) = it.next() {
                    if tok == "src" {
                        return it.next().map(str::to_string);
                    }
                }
                None
            })
    }

    pub fn connect(&self, addr: &str) -> Result<String> {
        let out = self.run(&["connect", addr])?;
        let t = out.trim().to_string();
        if t.contains("connected to") {
            Ok(t)
        } else {
            bail!("{t}")
        }
    }

    pub fn pair(&self, addr: &str, code: &str) -> Result<String> {
        let out = self.output(&["pair", addr, code])?;
        let t = format!(
            "{} {}",
            String::from_utf8_lossy(&out.stdout).trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        if t.contains("Successfully paired") {
            Ok(t.trim().to_string())
        } else {
            bail!("{}", t.trim())
        }
    }

    pub fn disconnect(&self, addr: &str) -> Result<()> {
        self.run(&["disconnect", addr]).map(|_| ())
    }

    /// Pasa un móvil conectado por USB a modo inalámbrico y se conecta a él.
    pub fn switch_to_wifi(&self, serial: &str) -> Result<String> {
        let ip = self
            .wifi_ip(serial)
            .context("no se encontró la IP del Wi-Fi: ¿está el móvil conectado a la misma red?")?;
        self.run_serial(serial, &["tcpip", "5555"])?;
        let addr = format!("{ip}:5555");
        // El demonio adb del móvil tarda un momento en reiniciarse en modo TCP.
        let mut last = None;
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(700));
            match self.connect(&addr) {
                Ok(msg) => return Ok(msg),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("no se pudo conectar a {addr}")))
    }
}

fn parse_inet(out: &str) -> Option<String> {
    let mut it = out.split_whitespace();
    while let Some(tok) = it.next() {
        if tok == "inet" {
            return it.next().and_then(|cidr| cidr.split('/').next()).map(str::to_string);
        }
    }
    None
}

pub fn parse_devices(out: &str) -> Vec<Device> {
    out.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?.to_string();
            let state = parts.next()?.to_string();
            let model = parts
                .find_map(|p| p.strip_prefix("model:"))
                .map(|m| m.replace('_', " "))
                .unwrap_or_default();
            Some(Device { serial, state, model })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_list() {
        let out = "List of devices attached\n\
                   R58M123ABC  device usb:1-1 product:beyond1 model:SM_G973F device:beyond1 transport_id:1\n\
                   192.168.1.40:5555 device product:x model:Pixel_7 device:panther transport_id:2\n\
                   ZY22 unauthorized usb:1-2 transport_id:3\n\n";
        let d = parse_devices(out);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].model, "SM G973F");
        assert!(d[0].state == "device" && !d[0].is_wireless());
        assert!(d[1].is_wireless());
        assert_eq!(d[2].state, "unauthorized");
        assert_eq!(d[2].label(), "ZY22");
    }

    #[test]
    fn parses_inet() {
        let out = "30: wlan0    inet 192.168.1.40/24 brd 192.168.1.255 scope global wlan0\\ valid_lft forever";
        assert_eq!(parse_inet(out).as_deref(), Some("192.168.1.40"));
    }
}
