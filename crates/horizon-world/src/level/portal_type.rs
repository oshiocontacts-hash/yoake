use super::Behaviours;
use crate::registry::BlockStateId;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortalSize {
    pub min_width: i32,
    pub max_width: i32,
    pub min_height: i32,
    pub max_height: i32,
}

impl Default for PortalSize {
    fn default() -> Self {
        Self {
            min_width: 2,
            max_width: 21,
            min_height: 3,
            max_height: 21,
        }
    }
}

impl PortalSize {
    pub const fn valid(self, width: i32, height: i32) -> bool {
        width >= self.min_width
            && width <= self.max_width
            && height >= self.min_height
            && height <= self.max_height
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortalIgnite {
    pub item: Option<String>,
    pub fluid: Option<String>,
    pub fire: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortalLook {
    pub tint: Option<String>,
    pub ambient_sound: Option<String>,
    pub travel_sound: Option<String>,
    pub particle: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortalType {
    pub id: String,
    pub slot: u16,
    pub frame: String,
    pub frame_members: Option<std::collections::BTreeSet<String>>,
    pub ignite: PortalIgnite,
    pub size: PortalSize,
    pub target: String,
    pub scale: f64,
    pub portal_block: String,
    pub look: PortalLook,
}

impl PortalType {
    pub fn parse(id: &str, json: &Value) -> Result<Self, String> {
        let object = json
            .as_object()
            .ok_or_else(|| "portal type must be a JSON object".to_owned())?;
        let frame = required_string(object, "frame")?;
        if frame == "#" || frame.is_empty() {
            return Err("frame must be a block id or #tag".into());
        }
        let target = required_string(object, "target")?;
        let portal_block = optional_string(object, "portal_block")?
            .unwrap_or_else(|| "minecraft:nether_portal".into());
        let scale = match object.get("scale") {
            None => 1.0,
            Some(value) => value
                .as_f64()
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .ok_or_else(|| "scale must be a finite number greater than zero".to_owned())?,
        };
        let size = parse_size(object.get("size"))?;
        let ignite = parse_ignite(object.get("ignite"))?;
        if ignite.item.is_none() && ignite.fluid.is_none() && !ignite.fire {
            return Err("ignite must enable item, fluid, or fire".into());
        }
        let look = parse_look(object.get("look"))?;
        Ok(Self {
            id: id.into(),
            slot: 0,
            frame,
            frame_members: None,
            ignite,
            size,
            target,
            scale,
            portal_block,
            look,
        })
    }

    pub fn is_frame(&self, state: BlockStateId, b: &Behaviours) -> bool {
        if let Some(members) = &self.frame_members {
            return members.contains(b.table.name(state));
        }
        match self.frame.strip_prefix('#') {
            Some(tag) => b.tag(tag).is_some_and(|set| b.in_set(state, set)),
            None => b
                .table
                .block_by_name(&self.frame)
                .is_some_and(|block| b.block(state) == block),
        }
    }

    pub fn is_fill(&self, state: BlockStateId, b: &Behaviours) -> bool {
        b.table
            .block_by_name(&self.portal_block)
            .is_some_and(|block| b.block(state) == block)
    }

    pub fn frame_state(&self, b: &Behaviours) -> Option<BlockStateId> {
        if let Some(members) = &self.frame_members {
            return Some(b.default_of(b.table.block_by_name(members.first()?)?));
        }
        let block = match self.frame.strip_prefix('#') {
            Some(tag) => b.tag(tag)?.first()?,
            None => b.table.block_by_name(&self.frame)?,
        };
        Some(b.default_of(block))
    }

    pub fn fill_state(&self, x_axis: bool, b: &Behaviours) -> Option<BlockStateId> {
        let base = b.default_of(b.table.block_by_name(&self.portal_block)?);
        Some(
            b.table
                .with_text(base, b.props.axis, if x_axis { "x" } else { "z" })
                .unwrap_or(base),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct PortalTypes {
    pub types: BTreeMap<String, PortalType>,
}

impl PortalTypes {
    pub fn get(&self, id: &str) -> Option<&PortalType> {
        self.types.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &PortalType> {
        self.types.values()
    }

    pub fn by_slot(&self, slot: u16) -> Option<&PortalType> {
        self.iter().find(|portal| portal.slot == slot)
    }

    pub fn for_fill(&self, state: BlockStateId, b: &Behaviours) -> Option<&PortalType> {
        self.iter().find(|portal| portal.is_fill(state, b))
    }
}

fn required_string(object: &serde_json::Map<String, Value>, key: &str) -> Result<String, String> {
    optional_string(object, key)?.ok_or_else(|| format!("{key} must be a nonempty string"))
}

fn optional_string(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        _ => Err(format!("{key} must be a nonempty string")),
    }
}

fn parse_size(value: Option<&Value>) -> Result<PortalSize, String> {
    let mut size = PortalSize::default();
    let Some(value) = value else { return Ok(size) };
    let object = value.as_object().ok_or("size must be an object")?;
    for (key, slot) in [
        ("min_width", &mut size.min_width),
        ("max_width", &mut size.max_width),
        ("min_height", &mut size.min_height),
        ("max_height", &mut size.max_height),
    ] {
        if let Some(value) = object.get(key) {
            *slot = i32::try_from(value.as_i64().ok_or("size limits must be integers")?)
                .map_err(|_| "size limit is out of range")?;
        }
    }
    if size.min_width < 1
        || size.min_height < 1
        || size.min_width > size.max_width
        || size.min_height > size.max_height
        || size.max_width > 256
        || size.max_height > 256
    {
        return Err("size needs 1 <= min <= max <= 256".into());
    }
    Ok(size)
}

fn parse_ignite(value: Option<&Value>) -> Result<PortalIgnite, String> {
    let Some(value) = value else { return Err("ignite is required".into()) };
    let object = value.as_object().ok_or("ignite must be an object")?;
    let fire = match object.get("fire") {
        None => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err("ignite.fire must be a boolean".into()),
    };
    Ok(PortalIgnite {
        item: optional_string(object, "item")?,
        fluid: optional_string(object, "fluid")?,
        fire,
    })
}

fn parse_look(value: Option<&Value>) -> Result<PortalLook, String> {
    let Some(value) = value else { return Ok(PortalLook::default()) };
    let object = value.as_object().ok_or("look must be an object")?;
    let look = PortalLook {
        tint: optional_string(object, "tint")?,
        ambient_sound: optional_string(object, "ambient_sound")?,
        travel_sound: optional_string(object, "travel_sound")?,
        particle: optional_string(object, "particle")?,
    };
    if look.tint.as_deref().is_some_and(|tint| {
        tint.len() != 7 || !tint.starts_with('#') || !tint[1..].chars().all(|c| c.is_ascii_hexdigit())
    }) {
        return Err("look.tint must be #rrggbb".into());
    }
    Ok(look)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid() -> Value {
        json!({"frame":"#example:frames","ignite":{"item":"minecraft:flint_and_steel"},
            "target":"example:moon","scale":8.0})
    }

    #[test]
    fn defaults_and_optional_fields_parse() {
        let portal = PortalType::parse("example:moon", &valid()).unwrap();
        assert_eq!(portal.size, PortalSize::default());
        assert_eq!(portal.portal_block, "minecraft:nether_portal");
        assert_eq!(portal.scale, 8.0);
        assert!(!portal.ignite.fire && portal.ignite.fluid.is_none());
    }

    #[test]
    fn invalid_sizes_scale_and_empty_ignition_are_rejected() {
        for size in [
            json!({"min_width":0}), json!({"min_width":4,"max_width":3}),
            json!({"min_height":5,"max_height":4}), json!({"max_width":257}),
        ] {
            let mut value = valid();
            value["size"] = size;
            assert!(PortalType::parse("example:bad", &value).unwrap_err().contains("size"));
        }
        for scale in [json!(0), json!(-1), json!("eight")] {
            let mut value = valid();
            value["scale"] = scale;
            assert!(PortalType::parse("example:bad", &value).is_err());
        }
        let mut value = valid();
        value["ignite"] = json!({});
        assert!(PortalType::parse("example:bad", &value).is_err());
    }

    #[test]
    fn look_is_validated_without_changing_travel_defaults() {
        let mut value = valid();
        value["look"] = json!({"tint":"#80c0Ff","travel_sound":"example:travel"});
        let portal = PortalType::parse("example:moon", &value).unwrap();
        assert_eq!(portal.look.tint.as_deref(), Some("#80c0Ff"));
        value["look"]["tint"] = json!("blue");
        assert!(PortalType::parse("example:bad", &value).is_err());
    }
}
