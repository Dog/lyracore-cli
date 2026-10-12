use lyracore_cli::{
    error::EXIT_OK, ops, proc::RealProcessRunner, project::ProjectLayout, Error, Result,
};

const USAGE: &str = "lyracore-ops: host administration\n\nUSAGE:\n  lyracore-ops service reconcile\n\nInstalls the Standalone Supervisor unit from the current Core checkout and restarts\nSpacetimeDB. Requires root. Does not fetch, reset or publish source.\nUpdate and review the checkout separately before running this command.";

fn run(args: &[String]) -> Result<()> {
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        []
        | ["help"]
        | ["--help"]
        | ["-h"]
        | ["service", "--help"]
        | ["service", "reconcile", "--help"] => {
            println!("{USAGE}");
            Ok(())
        }
        ["service", "reconcile"] => {
            ops::service::reconcile(&ProjectLayout::discover()?, &RealProcessRunner)
        }
        _ => Err(Error::Usage(USAGE.into())),
    }
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(()) => EXIT_OK,
        Err(error) => {
            eprintln!("lyracore-ops: {error}");
            error.exit_code()
        }
    };
    std::process::exit(code);
}
