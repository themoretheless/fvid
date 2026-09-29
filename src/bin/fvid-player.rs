//! Desktop entry point: opening the app starts the player without a subcommand.
fn main() {
    if let Err(error) = fvid::player::run(std::env::args().skip(1).collect()) {
        eprintln!("FVid: {error}");
        std::process::exit(1);
    }
}
