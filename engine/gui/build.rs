fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/main.slint", config).expect("Slint のコンパイルに失敗");

    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/QuicklIME.ico");
        res.compile().expect("Windows リソースのコンパイルに失敗");
    }
}
