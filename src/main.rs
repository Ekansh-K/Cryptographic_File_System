use clap::Parser;

fn main() -> anyhow::Result<()> {
    let args = cfs_io::cli::Cli::parse();
    cfs_io::cli::dispatch(args)
}
