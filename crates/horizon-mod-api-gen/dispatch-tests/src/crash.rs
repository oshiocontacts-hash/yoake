use std::cell::{Cell, RefCell};
use std::sync::Once;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeModCall {
    pub id: Option<&'static str>,
    pub index: usize,
    pub hook: &'static str,
}

thread_local! {
    static CURRENT: Cell<Option<NativeModCall>> = const { Cell::new(None) };
    static PANIC_MARKS: RefCell<Option<Vec<Option<NativeModCall>>>> = const { RefCell::new(None) };
}

pub struct NativeModGuard(Option<NativeModCall>);

pub fn enter_native_mod(call: NativeModCall) -> NativeModGuard {
    NativeModGuard(CURRENT.with(|current| current.replace(Some(call))))
}

impl Drop for NativeModGuard {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(self.0));
    }
}

pub fn native_mod_call() -> Option<NativeModCall> {
    CURRENT.with(Cell::get)
}

pub fn record_panic_marks() {
    install_hook();
    PANIC_MARKS.with(|marks| *marks.borrow_mut() = Some(Vec::new()));
}

pub fn take_panic_marks() -> Vec<Option<NativeModCall>> {
    PANIC_MARKS.with(|marks| marks.borrow_mut().take().unwrap_or_default())
}

fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic".to_owned()
    }
}

pub fn install_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            PANIC_MARKS.with(|marks| {
                if let Some(marks) = marks.borrow_mut().as_mut() {
                    marks.push(native_mod_call());
                }
            });
            if !mod_guard::capture_panic(info) {
                previous(info);
            }
        }));
    });
}

include!(concat!(env!("OUT_DIR"), "/mod_guard_runtime.rs"));
