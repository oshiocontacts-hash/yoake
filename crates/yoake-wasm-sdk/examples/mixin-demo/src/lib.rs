use yoake_wasm_sdk::{
    export_mixin,
    mixin::{Handler, HeadOutcome, Value},
};

pub const FALL: &str = "world::living_fall::calculate_fall_damage#9a9a629c2a289ab6ffb883cb0cd5891af74587992a6e2da94d8090723ede5b94";
pub const CROP: &str = "world::level::bonemeal::crop_bonemeal_increase#b61d671fb9375f084f2bf026f466add0e8423efd996180dee3fb52ca18348fd1";
pub const CHAT: &str = "game::chat::handle_chat_input#2d6b616020cec7006f8b8c6aeb2d8f9faf0caa7a8dbeb7f38b5a4d839b0d2e62";

fn halve_fall(result: Value) -> Option<Value> {
    match result {
        Value::Signed(damage) => Some(Value::Signed(damage / 2)),
        _ => None,
    }
}

fn double_growth(result: Value) -> Option<Value> {
    match result {
        Value::Signed(increase) => Some(Value::Signed(increase.saturating_mul(2))),
        _ => None,
    }
}

fn log_chat(arguments: &[Value]) -> HeadOutcome {
    if let Some(Value::Text(message)) = arguments.get(1) {
        println!("[mixin-demo] chat: {message}");
    }
    HeadOutcome::Proceed
}

static HANDLERS: &[Handler] = &[
    Handler {
        site: FALL,
        head: None,
        returned: Some(halve_fall),
        overwrite: None,
    },
    Handler {
        site: CROP,
        head: None,
        returned: Some(double_growth),
        overwrite: None,
    },
    Handler {
        site: CHAT,
        head: Some(log_chat),
        returned: None,
        overwrite: None,
    },
];

export_mixin!(HANDLERS);
