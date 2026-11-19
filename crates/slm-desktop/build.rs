//! Windows EXE resources (ICON + VERSIONINFO), following KohdaLab TRKR's
//! `trkr-desktop/build.rs`. MSVC targets use `rc.exe` (override with `RC`),
//! GNU targets use `windres` (override with `WINDRES`). Non-Windows targets
//! need no resource compiler; the window icon is embedded from
//! `assets/icon.png` in `main.rs`.

use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.png");
    if env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numeric = format!("{},0", version.split('-').next().unwrap().replace('.', ","));
    let (name, description) = ("kohdalab-slm", "KohdaLab SLM");
    let icon = manifest
        .join("assets/icon.ico")
        .display()
        .to_string()
        .replace('\\', "/");
    let rc = out.join(format!("{name}.rc"));
    fs::write(
        &rc,
        format!(
            r#"101 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904B0"
  BEGIN
   VALUE "CompanyName", "Kohdalab\0"
   VALUE "FileDescription", "{description}\0"
   VALUE "FileVersion", "{version}\0"
   VALUE "InternalName", "{name}\0"
   VALUE "OriginalFilename", "{name}.exe\0"
   VALUE "ProductName", "KohdaLab SLM\0"
   VALUE "ProductVersion", "{version}\0"
   VALUE "LegalCopyright", "Copyright KohdaLab contributors\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x409, 1200
 END
END
"#
        ),
    )
    .unwrap();
    let msvc = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default() == "msvc";
    let resource = out.join(format!("{name}.{}", if msvc { "res" } else { "o" }));
    let status = if msvc {
        Command::new(env::var("RC").unwrap_or_else(|_| "rc".into()))
            .arg("/nologo")
            .arg("/fo")
            .arg(&resource)
            .arg(&rc)
            .status()
    } else {
        Command::new(env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into()))
            .args(["-O", "coff", "-i"])
            .arg(&rc)
            .arg("-o")
            .arg(&resource)
            .status()
    }
    .expect("Windows resource compiler is required (rc.exe for MSVC, windres for GNU)");
    assert!(status.success(), "Windows resource compiler failed");
    println!("cargo:rustc-link-arg-bin={name}={}", resource.display());
}
