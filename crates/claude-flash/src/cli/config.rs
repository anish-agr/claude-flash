//! `flash config`: the settings file.

use std::fs;
use std::io;
use std::path::Path;

use clap::Subcommand;
use flash_core::config::{self, Config};

use super::{Env, Outcome, style};
use crate::{paths, store, system};

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Print the settings file's path
    Path,
    /// Print the settings file
    Show,
    /// Print one setting, such as `signals.done.color`
    Get { key: String },
    /// Change one setting, keeping the file's comments and layout
    Set { key: String, value: String },
    /// Open the settings file in an editor
    Edit,
    /// Check the settings file for mistakes
    Check,
    /// Replace the settings file with the commented defaults, keeping a backup
    Reset,
}

pub fn run(env: &Env, action: Option<ConfigAction>) -> Outcome {
    let path = env.paths.config_file();
    match action.unwrap_or(ConfigAction::Show) {
        ConfigAction::Path => println!("{}", path.display()),
        ConfigAction::Show => print!("{}", read_or_default(&path)?),
        ConfigAction::Get { key } => {
            let config = Config::parse(&read_or_default(&path)?).map_err(|e| e.to_string())?;
            println!("{}", config.get(&key).ok_or_else(|| format!("there is no setting called {key:?}"))?);
        }
        ConfigAction::Set { key, value } => {
            let updated = config::set_value(&read_or_default(&path)?, &key, &value).map_err(|e| e.to_string())?;
            store::write_atomic(&path, updated.as_bytes())
                .map_err(|e| format!("could not write {}: {e}", path.display()))?;
            let applied = Config::parse(&updated).ok().and_then(|c| c.get(&key)).unwrap_or(value);
            println!("{key} = {applied}");
            if env.client().health().is_ok() {
                println!("{}", style::dim("The running agent picks this up within a second."));
            }
        }
        ConfigAction::Edit => {
            if !path.exists() {
                store::write_atomic(&path, config::DEFAULT_TOML.as_bytes())
                    .map_err(|e| format!("could not create {}: {e}", path.display()))?;
            }
            system::edit(&path).map_err(|e| format!("could not open an editor: {e}"))?;
        }
        ConfigAction::Check => {
            Config::parse(&read_or_default(&path)?).map_err(|e| e.to_string())?;
            println!("{} is valid", paths::display(&path));
        }
        ConfigAction::Reset => {
            if path.exists() {
                let backup = path.with_extension("toml.bak");
                fs::copy(&path, &backup).map_err(|e| format!("could not back up {}: {e}", path.display()))?;
                println!("saved the previous settings as {}", paths::display(&backup));
            }
            store::write_atomic(&path, config::DEFAULT_TOML.as_bytes())
                .map_err(|e| format!("could not write {}: {e}", path.display()))?;
            println!("{} now holds the defaults", paths::display(&path));
        }
    }
    Ok(())
}

/// The file's text, or the commented defaults when there is no file yet.
fn read_or_default(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(config::DEFAULT_TOML.to_owned()),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}
