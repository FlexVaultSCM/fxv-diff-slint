fn main() {
    // Debug info is what makes elements findable by id and type name at runtime. Without it
    // every query returns nothing, which reads as a failing assertion rather than a missing
    // build flag.
    let config = slint_build::CompilerConfiguration::new().with_debug_info(true);
    slint_build::compile_with_config("ui/harness.slint", config).unwrap();
}
