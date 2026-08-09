fn main() {
    // `as_library` builds this crate as a Slint library rather than an application: no
    // MainWindow is required, and the generated types are declared public so a consuming
    // crate can name them. `rust_module` is the module the generated code lands in, which is
    // what a consumer's generated code refers to (`fxv_diff_slint::ui::CodeRow`).
    //
    // The library name must equal the `links` key in Cargo.toml exactly and contain no
    // hyphens; see the comment there.
    //
    // There are deliberately no include paths. A consuming crate re-parses this crate's
    // .slint sources using its own compiler configuration, so any path configured here does
    // not exist on their side. Every import has to resolve relative to the file making it,
    // and nothing under OUT_DIR can be imported at all.
    let config = slint_build::CompilerConfiguration::new()
        .as_library("FxvDiff")
        .rust_module("ui");

    slint_build::compile_with_config("ui/lib.slint", config).unwrap();
}
