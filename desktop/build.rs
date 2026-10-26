fn main() {
    slint_build::compile_with_config("ui/app.slint", slint_build::CompilerConfiguration::new().with_style("fluent".into()))
        .expect("compile ui");
    // Explorer/taskbar icon; resource ID 1 is also what the tray loads.
    winresource::WindowsResource::new().set_icon_with_id("ui/icons/app.ico", "1").compile().expect("embed icon");
}
