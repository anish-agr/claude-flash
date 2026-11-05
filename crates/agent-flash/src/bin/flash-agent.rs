// No console window: the agent runs in the background from login.
#![windows_subsystem = "windows"]

fn main() -> std::process::ExitCode {
    agent_flash::agent::main()
}
