// quicklime-llm (Vulkan 版)。処理の本体は lib.rs

fn main() -> std::process::ExitCode {
    quicklime_llm::run(quicklime_llm::Backend::Vulkan)
}
