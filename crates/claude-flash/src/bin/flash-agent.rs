// No console window: the agent runs in the background from login.
#![windows_subsystem = "windows"]

fn main() -> std::process::ExitCode {
    claude_flash::agent::main()
}
