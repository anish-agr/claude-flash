//! Push notifications to a phone, through ntfy or any JSON webhook.
//!
//! Requests go out through the system's `curl`, which ships with Windows 10 and later,
//! macOS and nearly every Linux distribution, so no TLS stack is bundled here. The
//! request, token included, is passed on curl's standard input rather than its
//! command line, where other local users could read it.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;

use flash_core::config::{Push, PushFormat};
use flash_core::engine::Notice;
use flash_core::event::Attention;
use serde_json::json;

use crate::{log, system};

pub fn send(push: &Push, notice: &Notice) {
    if !push.enabled() {
        return;
    }
    let request = curl_config(push, notice);
    let spawned = thread::Builder::new().name("push".into()).spawn(move || {
        if let Err(e) = run_curl(&request) {
            log!("push notification failed: {e}");
        }
    });
    if let Err(e) = spawned {
        log!("push notification not sent: {e}");
    }
}

fn curl_config(push: &Push, notice: &Notice) -> String {
    let mut lines = vec![
        format!("url = {}", quote(push.url.trim())),
        "request = \"POST\"".to_owned(),
        "silent".to_owned(),
        "show-error".to_owned(),
        "fail".to_owned(),
        "max-time = 20".to_owned(),
    ];
    let mut header = |value: String| lines.push(format!("header = {}", quote(&header_safe(&value))));
    if !push.token.trim().is_empty() {
        header(format!("Authorization: Bearer {}", push.token.trim()));
    }
    let body = match push.format {
        PushFormat::Ntfy => {
            header(format!("Title: {}", notice.title));
            header(format!("Priority: {}", if notice.kind.blocks_claude() { "high" } else { "default" }));
            header(format!("Tags: {}", tag(notice.kind)));
            notice.body.clone()
        }
        PushFormat::Json => {
            header("Content-Type: application/json".to_owned());
            json!({
                "kind": notice.kind,
                "title": notice.title,
                "body": notice.body,
                "ts": flash_core::time::rfc3339(system::unix_ms()),
            })
            .to_string()
        }
    };
    // `data-raw`, unlike `data`, never treats a leading `@` as a file to upload.
    lines.push(format!("data-raw = {}", quote(&body)));
    let mut config = lines.join("\n");
    config.push('\n');
    config
}

/// ntfy shows these tags as emoji.
fn tag(kind: Attention) -> &'static str {
    match kind {
        Attention::Done => "green_circle",
        Attention::Question => "large_blue_circle",
        Attention::Approval => "purple_circle",
        Attention::Error => "red_circle",
    }
}

/// Header values are single-line ASCII; anything else could split one header into two.
fn header_safe(value: &str) -> String {
    value.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() { c } else { ' ' }).collect()
}

/// A double-quoted string in curl's config-file syntax.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn run_curl(config: &str) -> Result<(), String> {
    let mut command = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    command.args(["--config", "-"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|e| format!("could not run curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(config.as_bytes()).map_err(|e| format!("could not pass the request to curl: {e}"))?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice() -> Notice {
        Notice { kind: Attention::Approval, title: "Claude needs approval".into(), body: "Bash in \"app\"\n@x".into() }
    }

    #[test]
    fn ntfy_requests_carry_title_priority_and_tag() {
        let push = Push { url: "https://ntfy.sh/topic".into(), token: "tk_1\r\nX-Evil: 1".into(), ..Push::default() };
        let config = curl_config(&push, &notice());
        assert!(config.contains("url = \"https://ntfy.sh/topic\""));
        assert!(config.contains("header = \"Title: Claude needs approval\""));
        assert!(config.contains("header = \"Priority: high\""));
        assert!(config.contains("header = \"Tags: purple_circle\""));
        assert!(config.contains("header = \"Authorization: Bearer tk_1  X-Evil: 1\""), "no header injection");
        assert!(config.contains(r#"data-raw = "Bash in \"app\"\n@x""#));
    }

    #[test]
    fn json_requests_are_one_object() {
        let push = Push { url: "https://example.com/hook".into(), format: PushFormat::Json, ..Push::default() };
        let config = curl_config(&push, &notice());
        let line = config.lines().find(|l| l.starts_with("data-raw")).unwrap();
        assert!(line.contains(r#"\"kind\":\"approval\""#));
        assert!(!config.contains("Authorization"));
    }
}
