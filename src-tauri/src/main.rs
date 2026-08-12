fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    // Headless entry points, so the scheduled agents never need a window.
    let outcome = if has("--sync-insights") {
        Some(socialflow_lib::sync_insights_headless())
    } else if has("--review") {
        Some(socialflow_lib::review_last_week())
    } else if has("--prepare-week") {
        Some(socialflow_lib::prepare_week_headless())
    } else if has("--strategy") {
        Some(socialflow_lib::show_strategy(has("--refresh")))
    } else {
        None
    };
    match outcome {
        Some(Ok(())) => (),
        Some(Err(error)) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
        None => socialflow_lib::run(),
    }
}
