use std::collections::HashSet;

use crate::hook_names::HOOK_NAMES;

#[derive(Debug, Eq, PartialEq)]
pub struct Subscriptions {
    hooks: HashSet<String>,
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

    pub fn subscribed(&self, hook: &str) -> bool {
        self.hooks.contains(hook)
    }
}
