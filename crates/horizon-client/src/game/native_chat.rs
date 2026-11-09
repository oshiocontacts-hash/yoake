use super::Game;
use horizon_client::mod_api;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NativeChatRecipients {
    #[default]
    All,
    Only(Vec<u128>),
}

impl NativeChatRecipients {
    pub(super) fn only(&self) -> Option<&[u128]> {
        match self {
            Self::All => None,
            Self::Only(players) => Some(players),
        }
    }
}

pub(super) fn dispatch_server_chat_listened(game: &Game<'_>, listened: &mut bool) {
    mod_api::dispatch_server_chat_listened(game, mod_api::mods(), listened);
}

pub(super) fn dispatch_server_chat(
    game: &mut Game<'_>,
    player: u128,
    name: &str,
    message: &mut String,
    recipients: &mut NativeChatRecipients,
    canceled: &mut bool,
) {
    mod_api::dispatch_server_chat(
        game,
        mod_api::mods(),
        player,
        name,
        message,
        recipients,
        canceled,
    );
}

pub(super) fn limit_chat_message(message: &mut String) {
    let mut units = 0;
    let cutoff = message.char_indices().find_map(|(index, character)| {
        units += character.len_utf16();
        (units > 256).then_some(index)
    });
    if let Some(index) = cutoff {
        message.truncate(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_recipients_allow_everyone() {
        assert_eq!(NativeChatRecipients::default().only(), None);
    }

    #[test]
    fn message_is_limited_without_splitting_a_surrogate_pair() {
        let mut unchanged = "a".repeat(256);
        limit_chat_message(&mut unchanged);
        assert_eq!(unchanged, "a".repeat(256));
        let mut rewritten = format!("{}😀extra", "a".repeat(255));
        limit_chat_message(&mut rewritten);
        assert_eq!(rewritten, "a".repeat(255));
    }
}
