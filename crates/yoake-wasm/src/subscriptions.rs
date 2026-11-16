use std::collections::HashSet;

use crate::full_hook_names::{FULL_HOOKS, HookSide};
use crate::hook_names::HOOK_NAMES;

#[derive(Debug, Eq, PartialEq)]
pub struct Subscriptions {
    hooks: HashSet<String>,
}

/// Which full-catalog world a module targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FullWorld {
    Client,
    Server,
}

impl Subscriptions {
    pub fn from_hook_names<I, S>(names: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut hooks = HashSet::new();
        for name in names {
            let name = name.as_ref();
            if !HOOK_NAMES.contains(&name) {
                return Err(format!("unknown hook subscription: {name}"));
            }
            hooks.insert(name.to_owned());
        }
        Ok(Self { hooks })
    }

    /// Subscriptions of a `client-full` or `server-full` module: every name must be a catalog hook
    /// that runs on that side and that this host has an adapter for.
    pub fn for_full_world<I, S>(world: FullWorld, names: I, unwired: &[&str]) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut hooks = HashSet::new();
        for name in names {
            let name = name.as_ref();
            let Some((_, side)) = FULL_HOOKS.iter().find(|(hook, _)| *hook == name) else {
                if world == FullWorld::Client && name == "gpu_frame" {
                    hooks.insert(name.to_owned());
                    continue;
                }
                return Err(format!("unknown hook subscription: {name}"));
            };
            let runs_here = matches!(
                (world, side),
                (_, HookSide::Both) | (FullWorld::Client, HookSide::Client) | (FullWorld::Server, HookSide::Server)
            );
            if !runs_here {
                let (here, there) = match world {
                    FullWorld::Client => ("client", "server"),
                    FullWorld::Server => ("server", "client"),
                };
                return Err(format!("hook {name} runs on the {there} side and cannot be subscribed from a {here} module"));
            }
            if unwired.contains(&name) {
                return Err(format!("hook {name} is not supported by this host"));
            }
            hooks.insert(name.to_owned());
        }
        Ok(Self { hooks })
    }

    pub fn subscribed(&self, hook: &str) -> bool {
        self.hooks.contains(hook)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.hooks.iter().map(String::as_str)
    }
}
