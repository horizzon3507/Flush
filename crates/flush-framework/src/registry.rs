//! Directory-tagged command registry (port of Yuzuha's `load`/`reload`).
//!
//! A Flush bot compiles its commands, so a "directory" is the label the
//! caller gives a group — generated projects map it to `src/commands`.
//! The registry holds the live command list behind a lock; the router
//! resolves each message against a fresh snapshot, so `load`/`reload`
//! take effect on a running bot. Loads are serialized and validated
//! before they commit: a failed reload changes nothing.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::command::{Command, CommandError};

struct Entry {
    /// Directory label; `None` for commands registered one by one.
    directory: Option<String>,
    command: Arc<dyn Command>,
}

fn names(command: &dyn Command) -> Vec<String> {
    [command.name()]
        .into_iter()
        .chain(command.aliases().iter().map(String::as_str))
        .map(|n| n.to_lowercase())
        .collect()
}

fn check_duplicates(commands: &[Arc<dyn Command>]) -> Result<(), CommandError> {
    let mut seen = HashSet::new();
    for command in commands {
        for name in names(command.as_ref()) {
            if !seen.insert(name.clone()) {
                return Err(CommandError::User(format!(
                    "duplicate command name or alias: {name}"
                )));
            }
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct CommandRegistry {
    entries: Mutex<Vec<Entry>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current command list, for routing and help snapshots.
    pub fn snapshot(&self) -> Vec<Arc<dyn Command>> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|e| e.command.clone())
            .collect()
    }

    /// Registers one standalone command (kept across `reload` of a
    /// directory, removed by `load`). Duplicate names are refused.
    pub fn add(&self, command: Box<dyn Command>) -> Result<(), CommandError> {
        let command: Arc<dyn Command> = command.into();
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut all: Vec<Arc<dyn Command>> = entries.iter().map(|e| e.command.clone()).collect();
        all.push(command.clone());
        check_duplicates(&all)?;
        entries.push(Entry {
            directory: None,
            command,
        });
        Ok(())
    }

    /// Replaces every command (manual and of other directories) with the
    /// ones from `directory` — mirrors Yuzuha `load(dir, replace = true)`.
    pub fn load(
        &self,
        directory: impl Into<String>,
        commands: Vec<Box<dyn Command>>,
    ) -> Result<(), CommandError> {
        let directory = directory.into();
        let loaded: Vec<Arc<dyn Command>> = commands.into_iter().map(Into::into).collect();
        check_duplicates(&loaded)?;
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        *entries = loaded
            .into_iter()
            .map(|command| Entry {
                directory: Some(directory.clone()),
                command,
            })
            .collect();
        Ok(())
    }

    /// Replaces only the commands of `directory`, keeping standalone
    /// commands and the ones loaded from other directories — mirrors
    /// Yuzuha `reload(dir)`.
    pub fn reload(
        &self,
        directory: impl AsRef<str>,
        commands: Vec<Box<dyn Command>>,
    ) -> Result<(), CommandError> {
        let directory = directory.as_ref();
        let loaded: Vec<Arc<dyn Command>> = commands.into_iter().map(Into::into).collect();
        check_duplicates(&loaded)?;
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        // Validate before committing: a refused reload changes nothing.
        check_duplicates(
            &entries
                .iter()
                .filter(|e| e.directory.as_deref() != Some(directory))
                .map(|e| e.command.clone())
                .chain(loaded.iter().cloned())
                .collect::<Vec<_>>(),
        )?;
        entries.retain(|e| e.directory.as_deref() != Some(directory));
        entries.extend(loaded.into_iter().map(|command| Entry {
            directory: Some(directory.to_string()),
            command,
        }));
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Cloneable handle to swap command groups on a running bot. A `reload`
/// after it returns is already visible to the next incoming message.
/// Obtained from [`crate::router::Bot::loader`].
#[derive(Clone)]
pub struct CommandLoader {
    registry: Arc<CommandRegistry>,
    // A shared slot, not a captured copy: help enabled on the bot after this
    // handle was created is still seen by `refresh`.
    help: Arc<Mutex<Option<Arc<crate::help::Help>>>>,
}

impl CommandLoader {
    pub(crate) fn new(
        registry: Arc<CommandRegistry>,
        help: Arc<Mutex<Option<Arc<crate::help::Help>>>>,
    ) -> Self {
        Self { registry, help }
    }

    fn refresh(&self) {
        let slot = self.help.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(help) = slot.as_ref() {
            help.refresh(&self.registry.snapshot());
        }
    }

    /// Replaces every command with the ones of `directory`.
    pub fn load(
        &self,
        directory: impl Into<String>,
        commands: Vec<Box<dyn Command>>,
    ) -> Result<(), CommandError> {
        self.registry.load(directory, commands)?;
        self.refresh();
        Ok(())
    }

    /// Replaces only the commands of `directory`, keeping manual
    /// registrations and other directories.
    pub fn reload(
        &self,
        directory: impl AsRef<str>,
        commands: Vec<Box<dyn Command>>,
    ) -> Result<(), CommandError> {
        self.registry.reload(directory, commands)?;
        self.refresh();
        Ok(())
    }

    /// Registers one standalone command.
    pub fn add(&self, command: Box<dyn Command>) -> Result<(), CommandError> {
        self.registry.add(command)?;
        self.refresh();
        Ok(())
    }

    /// The underlying registry, for direct reads (e.g. `snapshot`).
    pub fn registry(&self) -> Arc<CommandRegistry> {
        self.registry.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandCtx, CommandResult};
    use std::future::Future;

    struct Stub(&'static str, Vec<String>);

    impl Command for Stub {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn aliases(&self) -> &[String] {
            &self.1
        }
        fn execute<'a>(
            &'a self,
            _ctx: CommandCtx<'a>,
        ) -> std::pin::Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn stub(name: &'static str) -> Box<dyn Command> {
        Box::new(Stub(name, Vec::new()))
    }

    fn names_of(registry: &CommandRegistry) -> Vec<String> {
        let mut names: Vec<String> = registry
            .snapshot()
            .iter()
            .map(|c| c.name().to_string())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn load_replaces_everything() {
        let registry = CommandRegistry::new();
        registry.add(stub("manual")).unwrap();
        registry.load("a", vec![stub("one")]).unwrap();
        assert_eq!(names_of(&registry), ["one"]);
        registry.load("b", vec![stub("two"), stub("three")]).unwrap();
        assert_eq!(names_of(&registry), ["three", "two"]);
    }

    #[test]
    fn reload_preserves_manual_and_other_directories() {
        let registry = CommandRegistry::new();
        registry.add(stub("manual")).unwrap();
        registry.reload("a", vec![stub("one")]).unwrap();
        registry.reload("b", vec![stub("two")]).unwrap();
        registry.reload("a", vec![stub("three")]).unwrap();
        assert_eq!(names_of(&registry), ["manual", "three", "two"]);
    }

    #[test]
    fn duplicate_names_are_refused() {
        let registry = CommandRegistry::new();
        registry
            .load("a", vec![stub("dup"), stub("dup")])
            .unwrap_err();
        assert!(registry.is_empty());
        registry.load("a", vec![stub("one")]).unwrap();
        // Alias colliding with an existing name is also refused on reload.
        registry
            .reload("b", vec![Box::new(Stub("two", vec!["one".into()]))])
            .unwrap_err();
        assert_eq!(names_of(&registry), ["one"]);
    }

    #[test]
    fn loader_sees_help_enabled_after_its_creation() {
        let registry = Arc::new(CommandRegistry::new());
        let slot = Arc::new(Mutex::new(None));
        let loader = CommandLoader::new(registry, slot.clone());
        // `enable_help` runs after `loader()` on the bot: the handle must
        // still sync the snapshot.
        let help = Arc::new(crate::help::Help::new());
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(help.clone());
        loader.load("a", vec![stub("one")]).unwrap();
        assert_eq!(help.snapshot_names(), ["one"]);
    }

    #[test]
    fn reload_can_reuse_the_replaced_names() {
        let registry = CommandRegistry::new();
        registry.reload("a", vec![stub("one")]).unwrap();
        registry.reload("a", vec![stub("one"), stub("two")]).unwrap();
        assert_eq!(names_of(&registry), ["one", "two"]);
    }
}
