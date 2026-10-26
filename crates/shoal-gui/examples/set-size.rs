//! Resize a pane from outside a window, for manual checks until windows
//! size their panes (T43), such as the 200 × 60 redraw time:
//!
//! ```sh
//! cargo run --release -p shoal-gui --example set-size -- p3 200 60
//! ```

use std::process::ExitCode;

use shoal_client::{
    Client, NoAutoStart, Options,
    model::PaneId,
    proto::{Call, ClientKind},
};
use shoal_config::XdgPaths;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [pane, cols, rows] = args.as_slice() else {
        eprintln!("usage: set-size <pane> <cols> <rows>");
        return ExitCode::from(2);
    };
    match run(pane, cols, rows) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("set-size: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(pane: &str, cols: &str, rows: &str) -> Result<(), String> {
    let pane: PaneId = pane
        .parse()
        .map_err(|_| format!("{pane:?} isn't a pane id"))?;
    let cols = cols
        .parse()
        .map_err(|_| format!("{cols:?} isn't a column count"))?;
    let rows = rows
        .parse()
        .map_err(|_| format!("{rows:?} isn't a row count"))?;
    let paths = XdgPaths::from_env().map_err(|e| e.to_string())?;
    let socket = paths.socket_file().map_err(|e| e.to_string())?;
    let mut options = Options::new(socket, ClientKind::Cli);
    options.reconnect = false;
    let client = Client::connect(options, NoAutoStart)
        .map_err(|e| e.to_string())?
        .client;
    // Only the connection that last reported focus may size panes (§5.6).
    client
        .request(Call::WindowSetFocused { focused: true })
        .map_err(|e| e.to_string())?;
    let size = client
        .request(Call::PaneSetSize { pane, cols, rows })
        .map_err(|e| e.to_string())?;
    println!("{pane}: {size}");
    Ok(())
}
