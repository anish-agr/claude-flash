//! Starting the agent: one per user, listening on loopback, with the platform's
//! front end on the main thread and the policy engine on its own.

use std::fs;
use std::io;
use std::net::{Ipv4Addr, TcpListener};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc;

use flash_core::config::{Agent, Config};

use crate::api::Control;
use crate::client::Client;
use crate::hub::Hub;
use crate::paths::Paths;
use crate::runtime::{Request, Runtime, Shared};
use crate::server::Server;
use crate::ui::{self, Native};
use crate::{log, logging, store};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// No overlays, tray or menu bar; flashes are only logged.
    pub headless: bool,
    /// Listen here instead of on the configured port.
    pub port: Option<u16>,
    /// Copy the log to stderr, for an agent run from a terminal.
    pub echo: bool,
    /// Flip the on/off switch once running, and show that it happened.
    pub toggle: bool,
}

/// Entry point of the `flash-agent` executable.
pub fn main() -> ExitCode {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "toggle" => options.toggle = true,
            "--headless" => options.headless = true,
            "--foreground" => options.echo = true,
            "--port" => options.port = args.next().and_then(|p| p.parse().ok()),
            "--version" => {
                println!("flash-agent {}", flash_core::VERSION);
                return ExitCode::SUCCESS;
            }
            other => eprintln!("flash-agent: ignoring unknown argument {other:?}"),
        }
    }
    run(options)
}

pub fn run(options: Options) -> ExitCode {
    let paths = Paths::resolve();
    if let Err(e) = fs::create_dir_all(&paths.data_dir) {
        eprintln!("flash-agent: cannot create {}: {e}", paths.data_dir.display());
        return ExitCode::FAILURE;
    }
    logging::init(&paths.log_file(), options.echo);
    // The agent usually has no console, so a panic would otherwise leave no trace.
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log!("panic: {info}");
        report(info);
    }));
    let settings = configured_agent(&paths);
    let port = options.port.unwrap_or(settings.port);
    // Loopback keeps the agent on this machine. Listening wider is what lets Claude
    // Code in WSL or on another machine reach it, and the token then guards
    // everything, so a wider address is never a quieter door.
    let address = if settings.remote { Ipv4Addr::UNSPECIFIED } else { Ipv4Addr::LOCALHOST };

    let listener = match TcpListener::bind((address, port)) {
        Ok(listener) => listener,
        Err(e) => return step_aside(&paths, port, &options, &e),
    };
    let token = match store::load_or_create_token(&paths.token_file()) {
        Ok(token) => token,
        Err(e) => {
            log!("cannot create the API token at {}: {e}", paths.token_file().display());
            return ExitCode::FAILURE;
        }
    };

    let hub = Arc::new(Hub::default());
    let shared = Arc::new(Shared::default());
    let (requests, incoming) = mpsc::channel();
    if options.toggle {
        let _ = requests.send(Request::Control(Control::Toggle { confirm: true }, None));
    }
    let server = Server {
        listener,
        port,
        token,
        loopback_only: !settings.remote,
        requests: requests.clone(),
        hub: Arc::clone(&hub),
        shared: Arc::clone(&shared),
    };
    if let Err(e) = server.spawn() {
        log!("cannot start the HTTP server: {e}");
        return ExitCode::FAILURE;
    }

    let runtime_paths = paths.clone();
    let launch: ui::Launch =
        Box::new(move |display, ui, probe| Runtime::new(runtime_paths, port, display, ui, probe, hub, shared));
    let native = Native { requests, incoming, launch, paths };
    if options.headless { ui::headless::run(native) } else { ui::run_native(native) }
}

fn configured_agent(paths: &Paths) -> Agent {
    fs::read_to_string(paths.config_file())
        .ok()
        .and_then(|text| Config::parse(&text).ok())
        .map_or_else(|| Config::default().agent, |config| config.agent)
}

/// The port is taken. When another agent holds it, pass on any toggle and exit
/// quietly; anything else is a real failure.
fn step_aside(paths: &Paths, port: u16, options: &Options, bind_error: &io::Error) -> ExitCode {
    let client = Client::new(port, store::read_token(&paths.token_file()));
    match client.health() {
        Ok(health) => {
            if options.toggle
                && let Err(e) = client.control(&Control::Toggle { confirm: true })
            {
                log!("could not toggle the running agent: {e}");
            }
            if options.echo {
                log!("agent {} (pid {}) is already running on port {port}", health.version, health.pid);
            }
            ExitCode::SUCCESS
        }
        Err(_) => {
            log!("cannot listen on 127.0.0.1:{port}: {bind_error}");
            ExitCode::FAILURE
        }
    }
}
