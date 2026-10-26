//! Send requests to `shoald` from a script, for manual checks of things
//! the window or CLI can't do yet. Each pair of arguments is a method and
//! its JSON params, sent in order over one connection; each result prints.
//!
//! ```sh
//! cargo run --release -p shoal-gui --example call -- pane.zoom '{"pane":"p3","zoomed":true}'
//! ```
//!
//! Only the connection that last reported focus sizes panes (M1 spec §5.6),
//! so resizing from outside a window, say to time redraws at a size the
//! screen can't fit, takes two calls. The window takes the size back when
//! it next gains focus:
//!
//! ```sh
//! call window.set_focused '{"focused":true}' pane.set_size '{"pane":"p3","cols":200,"rows":60}'
//! ```

use std::process::ExitCode;

use serde_json::{Value, json};
use shoal_client::{
    Client, NoAutoStart, Options,
    proto::{Call, ClientKind},
};
use shoal_config::XdgPaths;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || !args.len().is_multiple_of(2) {
        eprintln!("usage: call <method> <params-json> [<method> <params-json> ...]");
        return ExitCode::from(2);
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("call: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let calls = args
        .chunks(2)
        .map(|pair| {
            let params: Value = serde_json::from_str(&pair[1])
                .map_err(|e| format!("{:?} isn't JSON: {e}", pair[1]))?;
            serde_json::from_value::<Call>(json!({"method": pair[0], "params": params}))
                .map_err(|e| format!("{} {}: {e}", pair[0], pair[1]))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let paths = XdgPaths::from_env().map_err(|e| e.to_string())?;
    let socket = paths.socket_file().map_err(|e| e.to_string())?;
    let mut options = Options::new(socket, ClientKind::Cli);
    options.reconnect = false;
    let client = Client::connect(options, NoAutoStart)
        .map_err(|e| e.to_string())?
        .client;
    for call in calls {
        let result = client.request(call).map_err(|e| e.to_string())?;
        println!("{result}");
    }
    Ok(())
}
