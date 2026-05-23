fn main() {
    tauri_build::build();

    // On Windows, embed a side-by-side manifest declaring a dependency on
    // common-controls v6 into ALL build targets — including unit-test
    // binaries. Without this the loader resolves to comctl32.dll v5 which
    // does not export `TaskDialogIndirect` (pulled in transitively by
    // tauri-plugin-dialog), and the test exe crashes with
    // STATUS_ENTRYPOINT_NOT_FOUND before main(). `tauri_build::build()`
    // already does this for the production binary via its generated rc, but
    // not for `cargo test`.
    #[cfg(target_os = "windows")]
    {
        let manifest = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let manifest_path = std::path::Path::new(&out_dir).join("test_manifest.xml");
        std::fs::write(&manifest_path, manifest).expect("write test_manifest.xml");
        // Only emit for the MSVC linker; gnu toolchains use a different
        // resource compilation path.
        let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
        if target_env == "msvc" {
            // `rustc-link-arg-tests` is accepted because we registered a
            // `[[test]]` target in Cargo.toml (tests/manifest_anchor.rs).
            // This covers unit-test binaries for lib AND bin, plus any
            // integration tests, but does NOT add to the production bin
            // (which already gets a manifest via tauri-build's resource.rc
            // — duplicating it would trigger linker CVT1100).
            println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
            println!(
                "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
                manifest_path.display()
            );
            // Examples: same need, no conflict (no resource.rc).
            println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
            println!(
                "cargo:rustc-link-arg-examples=/MANIFESTINPUT:{}",
                manifest_path.display()
            );
        }
    }
}
