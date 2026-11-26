//! Dual-mode entrypoint for `m3-nsh`: CLI subcommands run synchronously via `m3-cli`,
//! otherwise launches the NSH 0.7.0 Obsidian workbench.

use anyhow::Result;

#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;

struct DiagnosticLog;

impl log::Log for DiagnosticLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

fn is_platform_launch_flag(arg: &str) -> bool {
    arg.starts_with("-psn_") || arg.starts_with("-NS") || arg.starts_with("--nsh-")
}

fn main() -> Result<()> {
    static LOGGER: DiagnosticLog = DiagnosticLog;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Warn);

    let cli_args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !is_platform_launch_flag(a))
        .collect();
    if !cli_args.is_empty() {
        let code = m3_cli::run_cli_args(&cli_args)?;
        std::process::exit(code);
    }

    let exit_code = run_gui();
    if exit_code != std::process::ExitCode::SUCCESS {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_gui() -> std::process::ExitCode {
    platform::run_app(
        platform::AppSource::Rust(
            platform::Root::of::<m3_nsh::App>().app_id("systems.thinksmart.m3"),
        ),
        platform::AppOptions {
            title: "M3".to_owned(),
            app_id: "systems.thinksmart.m3".to_owned(),
            size: (1380, 880),
            keep_alive_on_close: true,
            ..Default::default()
        },
    )
}

#[cfg(not(target_os = "macos"))]
fn run_gui() -> std::process::ExitCode {
    eprintln!("m3-nsh GUI currently targets macOS (use CLI subcommands on this host).");
    std::process::ExitCode::FAILURE
}
