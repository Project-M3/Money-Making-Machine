use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = m3_cli::run_cli_args(&args)?;
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
