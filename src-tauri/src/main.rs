fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    // Headless entry points, so the scheduled agents never need a window.
    if args.iter().any(|a| a == "--prepare-week") {
        match socialflow_lib::prepare_week_headless() {
            Ok(()) => return,
            Err(error) => { eprintln!("Could not prepare the week: {error}"); std::process::exit(1); }
        }
    }
    if args.iter().any(|a| a == "--strategy") {
        let force = args.iter().any(|a| a == "--refresh");
        match socialflow_lib::show_strategy(force) {
            Ok(()) => return,
            Err(error) => { eprintln!("Could not read the strategy: {error}"); std::process::exit(1); }
        }
    }
    socialflow_lib::run()
}
