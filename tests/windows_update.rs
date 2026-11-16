#![cfg(windows)]

use std::fs;
use std::process::Command;

use base64::{engine::general_purpose::STANDARD, Engine};

#[test]
fn powershell_updater_replaces_both_executables() {
    let dir = tempfile::tempdir().unwrap();
    let stage = dir.path().join("staging folder");
    fs::create_dir(&stage).unwrap();
    let executable = env!("CARGO_BIN_EXE_odc");
    let mut previous = Command::new(executable).arg("--version").spawn().unwrap();
    let process_id = previous.id();
    assert!(previous.wait().unwrap().success());

    for name in ["odc.exe", "opendoc-cli.exe"] {
        fs::copy(executable, dir.path().join(name)).unwrap();
        fs::write(stage.join(name), format!("replacement for {name}")).unwrap();
    }

    let script: Vec<u8> = include_str!("../src/update_windows.ps1")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &STANDARD.encode(script),
        ])
        .env("ODC_UPDATE_PARENT", process_id.to_string())
        .env("ODC_UPDATE_DIR", dir.path())
        .env("ODC_UPDATE_STAGE", &stage)
        .env("ODC_UPDATE_CURRENT", env!("CARGO_PKG_VERSION"))
        .env("ODC_UPDATE_LATEST", "0.4.0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    for name in ["odc.exe", "opendoc-cli.exe"] {
        assert_eq!(
            fs::read_to_string(dir.path().join(name)).unwrap(),
            format!("replacement for {name}")
        );
    }
    assert!(!stage.exists());
}
