fn main() {
    // `socialflow --prepare-week` builds the next seven days with no window,
    // so the scheduled agent can run it whether or not the app is open.
    if std::env::args().any(|arg| arg == "--prepare-week") {
        match socialflow_lib::prepare_week_headless() {
            Ok(()) => return,
            Err(error) => {
                eprintln!("Could not prepare the week: {error}");
                std::process::exit(1);
            }
        }
    }
    socialflow_lib::run()
}
