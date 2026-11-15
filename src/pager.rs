//! Manejo transparente de paginación para terminal (less / pager).

use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

/// Muestra texto en la terminal usando un pager si el contenido excede la altura.
pub fn page_output(text: &str) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    if !stdout.is_terminal() {
        let mut lock = anstream::StripStream::new(stdout.lock());
        return ignore_broken_pipe(lock.write_all(text.as_bytes()).and_then(|()| lock.flush()));
    }

    // Comprobar la altura de la terminal
    let term_height = crossterm::terminal::size()
        .map(|(_, h)| h as usize)
        .unwrap_or(24);
    let line_count = text.lines().count();

    // Si cabe en pantalla, imprimir directamente
    if line_count <= term_height.saturating_sub(2) {
        let mut lock = stdout.lock();
        return ignore_broken_pipe(lock.write_all(text.as_bytes()).and_then(|()| lock.flush()));
    }

    // Intentar lanzar el paginador configurado o `less -R`
    let pager_cmd = std::env::var("PAGER").unwrap_or_else(|_| "less".to_string());
    let mut child = if pager_cmd.contains("less") {
        Command::new("less")
            .arg("-R") // Colores ANSI
            .arg("-F") // Salir si cabe en una pantalla
            .arg("-X") // No limpiar pantalla al salir
            .stdin(Stdio::piped())
            .spawn()
    } else {
        Command::new(&pager_cmd).stdin(Stdio::piped()).spawn()
    };

    if let Ok(ref mut proc) = child {
        if let Some(ref mut stdin) = proc.stdin {
            if stdin.write_all(text.as_bytes()).is_ok() && stdin.flush().is_ok() {
                // Dejar que el proceso termine
                drop(proc.stdin.take());
                let _ = proc.wait();
                return Ok(());
            }
        }
    }

    // Fallback: salida estándar directa si no hay pager disponible
    let mut lock = stdout.lock();
    lock.write_all(text.as_bytes())?;
    lock.flush()
}

fn ignore_broken_pipe(r: std::io::Result<()>) -> std::io::Result<()> {
    match r {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}
