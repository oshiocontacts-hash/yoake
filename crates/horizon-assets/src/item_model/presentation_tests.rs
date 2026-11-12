use super::*;
use std::cell::Cell;

#[derive(Default)]
struct Presentation {
    calls: Cell<usize>,
}

impl Presentation {
    fn check(&self, stack: &StackView, components: &Components, context: &Context<'_>) {
        assert_eq!(stack.item, "example:tank");
        assert_eq!(components.get("example:capacity"), Some(&Value::from(1000)));
        assert!(!components.contains_key("example:removed"));
        assert_eq!(context.display, DisplayContext::Gui);
        assert_eq!(context.seed, 37);
        self.calls.set(self.calls.get() + 1);
    }
}

impl ItemEnv for Presentation {
    fn item_model_number(
        &self,
        property: &str,
        stack: &StackView,
        components: &Components,
        context: &Context<'_>,
    ) -> f32 {
        self.check(stack, components, context);
        assert_eq!(property, "example:fill");
        components["example:amount"].as_f64().unwrap() as f32 / 1000.0
    }

    fn item_model_select(
        &self,
        property: &str,
        stack: &StackView,
        components: &Components,
        context: &Context<'_>,
    ) -> Option<Value> {
        self.check(stack, components, context);
        assert_eq!(property, "example:mode");
        components.get("example:mode").cloned()
    }

    fn item_tint(
        &self,
        source: &str,
        stack: &StackView,
        components: &Components,
        context: &Context<'_>,
    ) -> i32 {
        self.check(stack, components, context);
        assert_eq!(source, "example:fluid_color");
        components["example:color"].as_i64().unwrap() as i32
    }
}

fn models() -> ItemModels {
    let definition = ClientItem::decode(&serde_json::json!({"model": {
        "type": "minecraft:composite", "models": [
            {"type": "minecraft:range_dispatch", "property": "example:fill", "entries": [
                {"threshold": 0.5, "model": {"type": "minecraft:model", "model": "example:full"}}
            ], "fallback": {"type": "minecraft:model", "model": "example:empty"}},
            {"type": "minecraft:select", "property": "example:mode", "cases": [
                {"when": "output", "model": {"type": "minecraft:model", "model": "example:output"}}
            ], "fallback": {"type": "minecraft:model", "model": "example:input"}},
            {"type": "minecraft:model", "model": "example:fluid", "tints": [
                {"type": "example:fluid_color"}, {"type": "minecraft:constant", "value": -1}
            ]}
        ]
    }}))
    .unwrap();
    ItemModels {
        definitions: BTreeMap::from([("example:tank".into(), definition)]),
        prototypes: BTreeMap::from([(
            "example:tank".into(),
            BTreeMap::from([
                (
                    "minecraft:item_model".into(),
                    Value::String("example:tank".into()),
                ),
                ("example:capacity".into(), Value::from(1000)),
                ("example:amount".into(), Value::from(0)),
                ("example:mode".into(), Value::String("input".into())),
                ("example:color".into(), Value::from(0x123456)),
                ("example:removed".into(), Value::Bool(true)),
            ]),
        )]),
    }
}

fn context() -> Context<'static> {
    Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 37,
    }
}

fn model_names(item: &ResolvedItem) -> Vec<&str> {
    item.layers
        .iter()
        .map(|layer| match &layer.content {
            LayerContent::Model { model } => model.as_str(),
            other => panic!("not a model: {other:?}"),
        })
        .collect()
}

#[test]
fn components_switch_models_and_tint_individual_layers() {
    let models = models();
    let presentation = Presentation::default();
    let mut stack = StackView::new("example:tank", 1);
    stack.patch.insert("example:removed".into(), None);
    let empty = models.resolve(&stack, &context(), &presentation);
    assert_eq!(
        model_names(&empty),
        ["example:empty", "example:input", "example:fluid"]
    );
    assert_eq!(empty.layers[2].tints, [0x123456, -1]);
    stack
        .patch
        .insert("example:amount".into(), Some(Value::from(750)));
    stack
        .patch
        .insert("example:mode".into(), Some(Value::String("output".into())));
    stack
        .patch
        .insert("example:color".into(), Some(Value::from(0xabcdef)));
    let full = models.resolve(&stack, &context(), &presentation);
    assert_eq!(
        model_names(&full),
        ["example:full", "example:output", "example:fluid"]
    );
    assert_eq!(full.layers[2].tints, [0xabcdef, -1]);
    assert_eq!(presentation.calls.get(), 6);
}

#[test]
fn no_mod_keeps_zero_fallback_and_untinted_layers() {
    let models = models();
    let stack = StackView::new("example:tank", 1);
    let item = models.resolve(&stack, &context(), &NoEnv);
    assert_eq!(
        model_names(&item),
        ["example:empty", "example:input", "example:fluid"]
    );
    assert_eq!(item.layers[2].tints, [-1, -1]);
}

#[test]
fn vanilla_models_never_query_presentation_hooks() {
    let mut models = models();
    models.definitions.insert(
        "example:tank".into(),
        ClientItem::decode(&serde_json::json!({
            "model": {"type": "minecraft:range_dispatch", "property": "minecraft:count", "entries": [
                {"threshold": 0.0, "model": {"type": "minecraft:model", "model": "example:plain"}}
            ]}
        }))
        .unwrap(),
    );
    let presentation = Presentation::default();
    let item = models.resolve(
        &StackView::new("example:tank", 1),
        &context(),
        &presentation,
    );
    assert_eq!(model_names(&item), ["example:plain"]);
    assert_eq!(presentation.calls.get(), 0);
}
