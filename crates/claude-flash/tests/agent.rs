//! End to end: a headless agent on a private port and data directory, driven over
//! its real HTTP API and through the `flash` executable.

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use claude_flash::api::{Control, Status};
use claude_flash::client::Client;
use claude_flash::{journal, store};
use flash_core::event::Attention;
use flash_core::journal::{Channel, Record};
use serde_json::{Value, json};

struct Agent {
    child: Child,
    home: PathBuf,
    port: u16,
}

impl Agent {
    fn start(name: &str) -> Agent {
        let home = std::env::temp_dir().join(format!("claude-flash-it-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        let (child, port) = launch(&home, free_port());
        Agent { child, home, port }
    }

    /// Stops the agent and starts it again on the same data directory.
    fn restart(&mut self) {
        self.client().control(&Control::Quit).unwrap();
        self.child.wait().unwrap();
        (self.child, self.port) = launch(&self.home, self.port);
    }

    fn client(&self) -> Client {
        Client::new(self.port, store::read_token(&self.home.join("token")))
    }

    fn status(&self) -> Status {
        self.client().status().expect("status")
    }

    fn hook(&self, event: Value) {
        self.client().forward_hook(event.to_string().as_bytes(), "").expect("hook accepted");
    }

    fn flash(&self, args: &[&str]) -> Output {
        self.flash_with_input(args, None)
    }

    fn flash_with_input(&self, args: &[&str], input: Option<&str>) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_flash"))
            .args(args)
            .env("CLAUDE_FLASH_HOME", &self.home)
            .env("NO_COLOR", "1")
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run flash");
        if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
            stdin.write_all(input.as_bytes()).unwrap();
        }
        child.wait_with_output().unwrap()
    }

    fn journal(&self) -> Vec<Record> {
        journal::read(&self.home.join("journal"), 0)
    }

    fn wait_for(&self, what: &str, ready: impl Fn(&Agent) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.client().control(&Control::Quit);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.home);
    }
}

/// Starts a headless agent on `home` and waits until that very process answers.
///
/// A port that was free a moment ago can be taken before the agent binds it, most
/// often as the local end of another test's connection. The agent then exits, and
/// the next attempt uses a different port.
fn launch(home: &Path, mut port: u16) -> (Child, u16) {
    for _ in 0..5 {
        fs::write(home.join("config.toml"), format!("[agent]\nport = {port}\n")).unwrap();
        let mut child = spawn_agent(home);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if Client::new(port, None).health().is_ok_and(|health| health.pid == child.id()) {
                return (child, port);
            }
            if child.try_wait().unwrap().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        port = free_port();
    }
    panic!("could not start an agent for {}", home.display());
}

fn spawn_agent(home: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_flash-agent"))
        .arg("--headless")
        .env("CLAUDE_FLASH_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start flash-agent")
}

fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
}

fn event(name: &str, session: &str) -> Value {
    json!({ "hook_event_name": name, "session_id": session, "cwd": "/work/app", "permission_mode": "default" })
}

/// Sends raw bytes and returns the status code.
fn raw(port: u16, request: &str) -> u16 {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0)
}

#[test]
fn a_finished_turn_becomes_a_flash_and_a_journal_record() {
    let agent = Agent::start("done");
    agent.hook(event("UserPromptSubmit", "s1"));
    agent.hook(event("Stop", "s1"));
    agent.wait_for("the flash", |a| a.status().today.flashes == 1);
    assert_eq!(agent.status().today.done, 1);
    let stop = agent.journal().into_iter().find(|r| r.event == "Stop").expect("Stop is journaled");
    assert_eq!(stop.kind, Some(Attention::Done));
    assert_eq!(stop.delivered, [Channel::Flash]);
    assert_eq!(stop.project.as_deref(), Some("app"));
    assert_eq!(stop.path, None, "full paths are not stored by default");
}

#[test]
fn approvals_are_listed_as_waiting_until_the_tool_runs() {
    let agent = Agent::start("approval");
    agent.hook(event("UserPromptSubmit", "s1"));
    let mut request = event("PermissionRequest", "s1");
    request["tool_name"] = json!("Bash");
    request["tool_use_id"] = json!("toolu_1");
    agent.hook(request);
    agent.wait_for("the wait", |a| a.status().waiting.len() == 1);
    let wait = &agent.status().waiting[0];
    assert_eq!((wait.kind, wait.tool.as_deref()), (Attention::Approval, Some("Bash")));

    let mut done = event("PostToolUse", "s1");
    done["tool_use_id"] = json!("toolu_1");
    agent.hook(done);
    agent.wait_for("the wait to end", |a| a.status().waiting.is_empty());
    let resolved = agent.journal().into_iter().find(|r| r.waited_ms.is_some()).expect("the wait is journaled");
    assert_eq!(resolved.kind, Some(Attention::Approval));
}

#[test]
fn the_api_admits_only_local_non_browser_clients_with_the_token() {
    let agent = Agent::start("security");
    let port = agent.port;
    let host = format!("Host: 127.0.0.1:{port}\r\n");
    assert_eq!(raw(port, &format!("GET /v1/health HTTP/1.1\r\n{host}\r\n")), 200);
    assert_eq!(raw(port, &format!("GET /v1/status HTTP/1.1\r\n{host}\r\n")), 401);
    let token = store::read_token(&agent.home.join("token")).unwrap();
    assert_eq!(raw(port, &format!("GET /v1/status HTTP/1.1\r\n{host}Authorization: Bearer {token}\r\n\r\n")), 200);
    assert_eq!(raw(port, &format!("GET /v1/status HTTP/1.1\r\n{host}Authorization: Bearer {token}0\r\n\r\n")), 401);
    // A page in the browser, whatever it claims.
    let origin = format!(
        "POST /v1/control HTTP/1.1\r\n{host}Origin: https://example.com\r\nAuthorization: Bearer {token}\r\nContent-Length: 2\r\n\r\n{{}}"
    );
    assert_eq!(raw(port, &origin), 403);
    // DNS rebinding: the right port under someone else's name.
    assert_eq!(raw(port, &format!("GET /v1/health HTTP/1.1\r\nHost: evil.example:{port}\r\n\r\n")), 403);
    // A hook without the marker header Claude Flash installs.
    let body = event("Stop", "s1").to_string();
    let unmarked = format!("POST /v1/hooks/claude-code HTTP/1.1\r\n{host}Content-Length: {}\r\n\r\n{body}", body.len());
    assert_eq!(raw(port, &unmarked), 400);
    assert_eq!(raw(port, &format!("GET /nope HTTP/1.1\r\n{host}\r\n")), 404);
}

#[test]
fn sessions_run_with_claude_flash_off_are_ignored() {
    let agent = Agent::start("opt-out");
    let stop = event("Stop", "scripted").to_string();
    agent.client().forward_hook(stop.as_bytes(), "off").unwrap();
    agent.wait_for("the opt-out count", |a| a.status().opted_out == 1);
    assert_eq!(agent.status().today.flashes, 0);
    assert!(agent.journal().is_empty());
}

#[test]
fn switches_work_through_the_cli_and_survive_a_restart() {
    let mut agent = Agent::start("switches");
    let off = agent.flash(&["off"]);
    assert!(off.status.success());
    assert!(String::from_utf8_lossy(&off.stdout).contains("off"));
    assert!(!agent.status().enabled);

    let paused = agent.flash(&["pause", "15m"]);
    assert!(String::from_utf8_lossy(&paused.stdout).contains("off"), "off wins over paused");
    assert!(agent.status().paused_for_ms.is_some());
    assert!(!agent.flash(&["pause", "15"]).status.success(), "a bare number is refused");

    // Restart: the switch and the pause come back from disk.
    agent.restart();
    let status = agent.status();
    assert!(!status.enabled);
    assert!(status.paused_for_ms.is_some_and(|ms| ms > 14 * 60_000));

    assert!(agent.flash(&["on"]).status.success());
    assert!(agent.flash(&["resume"]).status.success());
    let status = agent.status();
    assert!(status.enabled && status.paused_for_ms.is_none());
}

#[test]
fn configuration_changes_apply_without_a_restart() {
    let agent = Agent::start("config");
    let set = agent.flash(&["config", "set", "signals.done.color", "#FFD400"]);
    assert!(set.status.success(), "{}", String::from_utf8_lossy(&set.stderr));
    assert!(String::from_utf8_lossy(&set.stdout).contains("within a second"));
    let remote = agent.flash(&["config", "set", "agent.remote", "false"]);
    assert!(
        String::from_utf8_lossy(&remote.stdout).contains("after `flash agent restart`"),
        "the agent's address changes only when it starts again"
    );
    let got = agent.flash(&["config", "get", "signals.done.color"]);
    assert_eq!(String::from_utf8_lossy(&got.stdout).trim(), "#FFD400");
    let text = fs::read_to_string(agent.home.join("config.toml")).unwrap();
    assert!(text.contains("port = "), "the rest of the file is kept");

    let bad = agent.flash(&["config", "set", "flash.hold_ms", "long"]);
    assert!(!bad.status.success());
    fs::write(agent.home.join("config.toml"), "[flash]\nhold_ms = \"long\"\n").unwrap();
    agent.wait_for("the agent to report the bad file", |a| a.status().config_error.is_some());
    fs::write(agent.home.join("config.toml"), format!("[agent]\nport = {}\n", agent.port)).unwrap();
    agent.wait_for("the agent to accept the fixed file", |a| a.status().config_error.is_none());
}

#[test]
fn hooks_install_into_a_settings_file_and_come_out_cleanly() {
    let agent = Agent::start("hooks");
    let settings = agent.home.join("settings.json");
    fs::write(
        &settings,
        r#"{"theme": "dark", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "other-tool"}]}]}}"#,
    )
    .unwrap();
    let path = settings.to_str().unwrap();
    assert!(agent.flash(&["hooks", "install", "--settings", path]).status.success());
    let installed: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(installed["theme"], "dark");
    assert_eq!(installed["hooks"]["Stop"][0]["hooks"][0]["command"], "other-tool");
    let ensure = &installed["hooks"]["SessionStart"][0]["hooks"][0];
    assert_eq!(ensure["args"], json!(["agent", "ensure"]));
    assert!(Path::new(ensure["command"].as_str().unwrap()).is_file(), "the hook runs this very flash binary");
    let status = agent.flash(&["hooks", "status", "--settings", path]);
    assert!(String::from_utf8_lossy(&status.stdout).contains("installed"));

    assert!(agent.flash(&["hooks", "uninstall", "--settings", path]).status.success());
    let removed: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(
        removed,
        json!({"theme": "dark", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "other-tool"}]}]}})
    );
}

#[test]
fn the_session_start_hook_and_version_1_verbs_reach_the_agent() {
    let agent = Agent::start("ensure");
    let start = agent.flash_with_input(&["agent", "ensure"], Some(&event("SessionStart", "s9").to_string()));
    assert!(start.status.success());
    assert!(start.stdout.is_empty(), "SessionStart output would become part of Claude's context");
    // Version 1 hooks ran `flash done --bg --require_session` with the event on stdin.
    let legacy = agent.flash_with_input(&["done", "--bg", "--require_session"], Some(&event("Stop", "s9").to_string()));
    assert!(legacy.status.success());
    agent.wait_for("both events", |a| {
        let events: Vec<String> = a.journal().into_iter().map(|r| r.event).collect();
        events.contains(&"SessionStart".to_owned()) && events.contains(&"Stop".to_owned())
    });
}

#[test]
fn a_second_agent_steps_aside() {
    let agent = Agent::start("single");
    let mut second = spawn_agent(&agent.home);
    let deadline = Instant::now() + Duration::from_secs(10);
    let code = loop {
        if let Some(status) = second.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "the second agent kept running");
        thread::sleep(Duration::from_millis(50));
    };
    assert!(code.success());
    assert!(agent.client().health().is_ok());
}

#[test]
fn signals_from_other_tools_and_the_event_stream() {
    let agent = Agent::start("signals");
    let out = agent.flash(&["signal", "error", "--title", "Nightly build failed", "--source", "ci"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    agent.wait_for("the signal", |a| a.status().today.error == 1);
    let events = agent.client().events(None, Duration::ZERO).unwrap();
    let record = events.records.iter().find(|r| r.event == "signal").expect("the stream has the signal");
    assert_eq!(record.source.as_deref(), Some("ci"));

    let stats = agent.flash(&["stats", "--json"]);
    let summary: Value = serde_json::from_slice(&stats.stdout).unwrap();
    assert_eq!(summary["signals"]["error"]["total"], 1);
}

#[test]
fn running_a_command_signals_how_it_went_and_keeps_its_exit_code() {
    let agent = Agent::start("run");
    let exe = env!("CARGO_BIN_EXE_flash");

    let passed = agent.flash(&["run", "--", exe, "--version"]);
    assert!(passed.status.success(), "{}", String::from_utf8_lossy(&passed.stderr));
    agent.wait_for("the done signal", |a| a.status().today.done == 1);

    let failed = agent.flash(&["run", "--", exe, "--not-a-flag"]);
    assert_eq!(failed.status.code(), Some(2), "the command's own exit code comes through");
    agent.wait_for("the error signal", |a| a.status().today.error == 1);

    let run: Vec<Record> = agent.journal().into_iter().filter(|r| r.source.as_deref() == Some("run")).collect();
    assert_eq!(run.len(), 2);
    assert_eq!(run[0].kind, Some(Attention::Done));
    assert_eq!(run[1].kind, Some(Attention::Error));

    let quiet = agent.flash(&["run", "--only-errors", "--", exe, "--version"]);
    assert!(quiet.status.success());
    assert_eq!(agent.status().today.done, 1, "--only-errors says nothing about a command that passed");
}
