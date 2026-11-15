//! Portapapeles robusto: en Linux `arboard` pierde el contenido al
//! soltar el handle, así que se usan primero las utilidades del sistema y,
//! como último recurso, la secuencia OSC 52 (funciona también por SSH).

use std::io::Write;
use std::process::{Command, Stdio};

fn pipe_to(cmd: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(text.as_bytes()).is_err() {
            return false;
        }
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Copia `text` al portapapeles. Devuelve el método usado.
pub fn copy(text: &str) -> Result<&'static str, String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && pipe_to("wl-copy", &[], text) {
            return Ok("wl-copy");
        }
        if std::env::var_os("DISPLAY").is_some() {
            if pipe_to("xclip", &["-selection", "clipboard"], text) {
                return Ok("xclip");
            }
            if pipe_to("xsel", &["--clipboard", "--input"], text) {
                return Ok("xsel");
            }
        }
        // OSC 52: el terminal se encarga (ssh, tmux con allow-passthrough…).
        let seq = format!("\x1b]52;c;{}\x07", base64(text.as_bytes()));
        let mut out = std::io::stdout();
        if out
            .write_all(seq.as_bytes())
            .and_then(|()| out.flush())
            .is_ok()
        {
            return Ok("OSC 52");
        }
        Err("no hay wl-copy, xclip ni xsel".into())
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = pipe_to;
        let _ = base64;
        arboard::Clipboard::new()
            .and_then(|mut c| c.set_text(text.to_string()))
            .map(|()| "portapapeles")
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_works() {
        assert_eq!(super::base64(b"Man"), "TWFu");
        assert_eq!(super::base64(b"Ma"), "TWE=");
        assert_eq!(super::base64(b"M"), "TQ==");
    }
}
