use clap::Parser;

/// Rootless toolkit for stock Android TVs.
#[derive(Debug, Parser)]
#[command(name = "rusttv", version = rusttv_core::VERSION, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
