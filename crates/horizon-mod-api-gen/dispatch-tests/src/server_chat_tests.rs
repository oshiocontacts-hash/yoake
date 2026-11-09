mod server_chat_tests {
    use super::*;

    struct ChatMod {
        cancel: bool,
        system: bool,
    }

    impl Hooks for ChatMod {
        fn server_chat_listened(&self, _: &dyn Host, event: &mut ServerChatListenedEvent) {
            event.listened = true;
        }

        fn server_chat(&self, _: &mut dyn Host, event: &mut ServerChatEvent<'_>) -> Flow {
            assert_eq!(event.player, 1);
            assert_eq!(event.name, "Guest");
            *event.message = "rewritten".to_owned();
            *event.recipients = NativeChatRecipients::Only(vec![2]);
            event.system = self.system;
            if self.cancel {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    #[test]
    fn no_mod_keeps_chat_and_the_listener_gate_unchanged() {
        let mods: Vec<Box<dyn Hooks>> = Vec::new();
        let mut listened = false;
        dispatch_server_chat_listened(&TestHost, &mods, &mut listened);
        assert!(!listened);

        let mut message = "original".to_owned();
        let mut recipients = NativeChatRecipients::All;
        let mut system = false;
        let mut canceled = false;
        dispatch_server_chat(
            &mut TestHost,
            &mods,
            1,
            "Guest",
            &mut message,
            &mut recipients,
            &mut system,
            &mut canceled,
        );
        assert_eq!(message, "original");
        assert_eq!(recipients, NativeChatRecipients::All);
        assert!(!system);
        assert!(!canceled);
    }

    #[test]
    fn guest_chat_listener_rewrites_and_limits_recipients() {
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(ChatMod {
            cancel: false,
            system: true,
        })];
        let mut listened = false;
        dispatch_server_chat_listened(&TestHost, &mods, &mut listened);
        assert!(listened);

        let mut message = "original".to_owned();
        let mut recipients = NativeChatRecipients::All;
        let mut system = false;
        let mut canceled = false;
        dispatch_server_chat(
            &mut TestHost,
            &mods,
            1,
            "Guest",
            &mut message,
            &mut recipients,
            &mut system,
            &mut canceled,
        );
        assert_eq!(message, "rewritten");
        assert_eq!(recipients, NativeChatRecipients::Only(vec![2]));
        assert!(system);
        assert!(!canceled);
    }

    #[test]
    fn guest_chat_listener_cancels_delivery() {
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(ChatMod {
            cancel: true,
            system: false,
        })];
        let mut message = "original".to_owned();
        let mut recipients = NativeChatRecipients::All;
        let mut system = false;
        let mut canceled = false;
        dispatch_server_chat(
            &mut TestHost,
            &mods,
            1,
            "Guest",
            &mut message,
            &mut recipients,
            &mut system,
            &mut canceled,
        );
        assert!(canceled);
    }
}
