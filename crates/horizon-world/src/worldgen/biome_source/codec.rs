//! Registry-bound biome-source definitions. Inline full biome objects are separate.
use crate::worldgen::{
    DecodeError, Object, RawValue,
    climate::{ParameterPoint, decode_parameter_point, unquantize},
    density::identifier,
    int_range, optional, required,
};
use super::grid::{GridBiomeSource, GridError};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BiomeSet {
    Direct(Vec<String>),
    Tag { name: String, members: Vec<String> },
}
impl BiomeSet {
    pub fn members(&self) -> &[String] {
        match self {
            Self::Direct(values) => values,
            Self::Tag { members, .. } => members,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BiomeSourceDefinition {
    Fixed(String),
    Checkerboard { biomes: BiomeSet, scale: i32 },
    End,
    MultiNoise(Vec<(ParameterPoint, String)>),
    Preset { name: String, inline: bool },
    /// `yoake:grid`: a rectangle of biome cells (see `GridBiomeSource`).
    /// `seed` is carried for the mod generator that built the grid; the
    /// lookup does not depend on it.
    Grid {
        cell_size: i32,
        origin: [i32; 2],
        width: i32,
        depth: i32,
        cells: Vec<String>,
        outside: String,
        seed: i64,
    },
}
#[derive(Clone, Debug)]
pub struct BiomeSourceCatalog {
    biomes: BTreeSet<String>,
    tags: BTreeMap<String, Vec<String>>,
    presets: BTreeSet<String>,
}
impl BiomeSourceCatalog {
    pub fn new(
        biomes: &BTreeSet<String>,
        tags: &BTreeMap<String, Vec<String>>,
        presets: &BTreeSet<String>,
    ) -> Result<Self, DecodeError> {
        if biomes.len() > 8192
            || tags.len() > 8192
            || presets.len() > 8192
            || tags.values().map(Vec::len).sum::<usize>() > 65536
        {
            return Err(DecodeError::Field("biome source catalog size"));
        }
        let biomes = biomes
            .iter()
            .map(|v| identifier(v))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let presets = presets
            .iter()
            .map(|v| identifier(v))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut resolved = BTreeMap::new();
        for (name, members) in tags {
            let members = members
                .iter()
                .map(|v| identifier(v))
                .collect::<Result<Vec<_>, _>>()?;
            if members.iter().any(|v| !biomes.contains(v)) {
                return Err(DecodeError::Field("unknown biome tag member"));
            }
            if resolved.insert(identifier(name)?, members).is_some() {
                return Err(DecodeError::Field("duplicate biome tag"));
            }
        }
        Ok(Self {
            biomes,
            tags: resolved,
            presets,
        })
    }
    fn biome(&self, raw: &RawValue) -> Result<String, DecodeError> {
        let name = identifier(&serde_json::from_str::<String>(raw.get())?)?;
        if !self.biomes.contains(&name) {
            return Err(DecodeError::Field("unknown biome"));
        }
        Ok(name)
    }
    fn set(&self, raw: &RawValue) -> Result<BiomeSet, DecodeError> {
        if raw.get().starts_with('"') {
            let name: String = serde_json::from_str(raw.get())?;
            if let Some(name) = name.strip_prefix('#') {
                let name = identifier(name)?;
                let members = self
                    .tags
                    .get(&name)
                    .ok_or(DecodeError::Field("unknown biome tag"))?
                    .clone();
                return Ok(BiomeSet::Tag { name, members });
            }
            return Ok(BiomeSet::Direct(vec![self.biome(raw)?]));
        }
        let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
        if values.len() > 8192 {
            return Err(DecodeError::Field("biome holder set size"));
        }
        Ok(BiomeSet::Direct(
            values
                .iter()
                .map(|v| self.biome(v))
                .collect::<Result<_, _>>()?,
        ))
    }
    fn inline(&self, raw: &RawValue) -> Result<BiomeSourceDefinition, DecodeError> {
        let entries: Vec<Object> = serde_json::from_str(raw.get())?;
        if entries.is_empty() || entries.len() > 65536 {
            return Err(DecodeError::Field("multi-noise entry count"));
        }
        let mut result = Vec::with_capacity(entries.len());
        for entry in entries {
            result.push((
                decode_parameter_point(required(&entry, "parameters")?.get())?,
                self.biome(required(&entry, "biome")?)?,
            ));
        }
        Ok(BiomeSourceDefinition::MultiNoise(result))
    }
    fn grid(&self, object: &Object) -> Result<BiomeSourceDefinition, DecodeError> {
        let cell_size = int_range(required(object, "cell_size")?, "grid cell_size", 4, 1 << 20)?;
        let origin = match optional(object, "origin") {
            None => [0, 0],
            Some(raw) => {
                let v: Vec<i32> = serde_json::from_str(raw.get())?;
                <[i32; 2]>::try_from(v).map_err(|_| DecodeError::Field("grid origin"))?
            }
        };
        let width = int_range(required(object, "width")?, "grid width", 1, 256)?;
        let depth = int_range(required(object, "depth")?, "grid depth", 1, 256)?;
        let raw_cells: Vec<Box<RawValue>> = serde_json::from_str(required(object, "cells")?.get())?;
        if raw_cells.len() != (width as usize) * (depth as usize) {
            return Err(DecodeError::Field("grid cells"));
        }
        let cells = raw_cells
            .iter()
            .map(|v| self.biome(v))
            .collect::<Result<Vec<_>, _>>()?;
        let outside = self.biome(required(object, "outside")?)?;
        let seed = match optional(object, "seed") {
            None => 0,
            Some(raw) => serde_json::from_str::<i64>(raw.get())?,
        };
        // Shape rules (cell size and origin multiples of 4) live in one place.
        GridBiomeSource::new(cell_size, origin, width, depth, &cells, &outside)
            .map_err(|e| DecodeError::Field(match e {
                GridError::CellSize => "grid cell_size must be a multiple of 4",
                GridError::Origin => "grid origin must be multiples of 4",
                GridError::Extent => "grid width and depth",
                GridError::CellCount => "grid cells",
                GridError::TooManyBiomes => "grid biomes",
            }))?;
        Ok(BiomeSourceDefinition::Grid {
            cell_size,
            origin,
            width,
            depth,
            cells,
            outside,
            seed,
        })
    }
    pub fn decode(&self, json: &str) -> Result<BiomeSourceDefinition, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        Ok(match kind.as_str() {
            "minecraft:fixed" => {
                BiomeSourceDefinition::Fixed(self.biome(required(&object, "biome")?)?)
            }
            "minecraft:checkerboard" => BiomeSourceDefinition::Checkerboard {
                biomes: self.set(required(&object, "biomes")?)?,
                scale: optional(&object, "scale")
                    .map_or(Ok(2), |v| int_range(v, "biome scale", 0, 62))?,
            },
            "minecraft:the_end" => {
                for name in [
                    "the_end",
                    "end_highlands",
                    "end_midlands",
                    "small_end_islands",
                    "end_barrens",
                ] {
                    if !self.biomes.contains(&format!("minecraft:{name}")) {
                        return Err(DecodeError::Field("missing End biome"));
                    }
                }
                BiomeSourceDefinition::End
            }
            "minecraft:multi_noise" => {
                // mapEither prefers a successful inline list, then tries preset.
                if let Some(raw) = optional(&object, "biomes") {
                    if let Ok(value) = self.inline(raw) {
                        return Ok(value);
                    }
                }
                let raw = required(&object, "preset")?;
                if raw.get().starts_with('"') {
                    let name = identifier(&serde_json::from_str::<String>(raw.get())?)?;
                    if !self.presets.contains(&name) {
                        return Err(DecodeError::Field("unknown biome preset holder"));
                    }
                    BiomeSourceDefinition::Preset {
                        name,
                        inline: false,
                    }
                } else {
                    let preset: Object = serde_json::from_str(raw.get())?;
                    let name = identifier(&serde_json::from_str::<String>(
                        required(&preset, "preset")?.get(),
                    )?)?;
                    if !matches!(name.as_str(), "minecraft:overworld" | "minecraft:nether") {
                        return Err(DecodeError::Field("unknown built-in biome preset"));
                    }
                    BiomeSourceDefinition::Preset { name, inline: true }
                }
            }
            "yoake:grid" => self.grid(&object)?,
            _ => return Err(DecodeError::Field("unknown biome source")),
        })
    }
    /// Encode definitions in the vanilla canonical shape; decoding the result
    /// validates registry references and bounds before publishing JSON.
    pub fn encode(&self, definition: &BiomeSourceDefinition) -> Result<String, DecodeError> {
        if let BiomeSourceDefinition::Grid {
            cell_size,
            origin,
            width,
            depth,
            cells,
            outside,
            seed,
        } = definition
        {
            let json = format!(
                "{{\"type\":\"yoake:grid\",\"cell_size\":{cell_size},\"origin\":[{},{}],\"width\":{width},\"depth\":{depth},\"cells\":{},\"outside\":{},\"seed\":{seed}}}",
                origin[0],
                origin[1],
                serde_json::to_string(cells)?,
                quoted(outside)
            );
            self.decode(&json)?;
            return Ok(json);
        }
        let (kind, fields) = match definition {
            BiomeSourceDefinition::Fixed(name) => ("fixed", format!(",\"biome\":{}", quoted(name))),
            BiomeSourceDefinition::End => ("the_end", String::new()),
            BiomeSourceDefinition::Checkerboard { biomes, scale } => {
                let values = match biomes {
                    BiomeSet::Tag { name, .. } => quoted(&format!("#{name}")),
                    BiomeSet::Direct(values) if values.len() == 1 => quoted(&values[0]),
                    BiomeSet::Direct(values) => serde_json::to_string(values)?,
                };
                (
                    "checkerboard",
                    format!(
                        ",\"biomes\":{values}{}",
                        if *scale == 2 {
                            String::new()
                        } else {
                            format!(",\"scale\":{scale}")
                        }
                    ),
                )
            }
            BiomeSourceDefinition::MultiNoise(entries) => {
                let values = entries
                    .iter()
                    .map(|(point, name)| {
                        format!(
                            "{{\"parameters\":{},\"biome\":{}}}",
                            point_json(point),
                            quoted(name)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                ("multi_noise", format!(",\"biomes\":[{values}]"))
            }
            BiomeSourceDefinition::Grid { .. } => unreachable!("encoded above"),
            BiomeSourceDefinition::Preset { name, inline } => {
                let value = if *inline {
                    format!("{{\"preset\":{}}}", quoted(name))
                } else {
                    quoted(name)
                };
                ("multi_noise", format!(",\"preset\":{value}"))
            }
        };
        let json = format!("{{\"type\":\"minecraft:{kind}\"{fields}}}");
        self.decode(&json)?;
        Ok(json)
    }
}
fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("string serializes")
}
fn point_json(point: &ParameterPoint) -> String {
    let names = [
        "temperature",
        "humidity",
        "continentalness",
        "erosion",
        "depth",
        "weirdness",
    ];
    let mut fields = Vec::with_capacity(7);
    for (name, p) in names.into_iter().zip(point.coordinates) {
        let min = serde_json::to_string(&unquantize(p.min)).expect("finite coordinate");
        let max = serde_json::to_string(&unquantize(p.max)).expect("finite coordinate");
        let value = if min == max {
            min
        } else {
            format!("[{min},{max}]")
        };
        fields.push(format!("{}:{value}", quoted(name)));
    }
    fields.push(format!(
        "\"offset\":{}",
        serde_json::to_string(&unquantize(point.offset)).expect("finite offset")
    ));
    format!("{{{}}}", fields.join(","))
}

#[cfg(test)]
mod grid_tests {
    use super::*;
    use crate::worldgen::biome_source::ResolvedBiomeSource;

    fn catalog() -> BiomeSourceCatalog {
        let biomes: BTreeSet<String> = ["minecraft:plains", "minecraft:desert", "minecraft:ocean"]
            .into_iter()
            .map(String::from)
            .collect();
        BiomeSourceCatalog::new(&biomes, &BTreeMap::new(), &BTreeSet::new()).unwrap()
    }
    const GRID: &str = r#"{"type":"yoake:grid","cell_size":16,"origin":[-16,-16],"width":2,"depth":1,
        "cells":["minecraft:plains","minecraft:desert"],"outside":"minecraft:ocean","seed":-5}"#;

    #[test]
    fn decodes_and_samples() {
        let def = catalog().decode(GRID).unwrap();
        let ResolvedBiomeSource::Grid(src) = def.resolve(&BTreeMap::new()).unwrap() else {
            panic!("not a grid")
        };
        assert_eq!(src.noise_biome([-4, 0, -4]), "minecraft:plains");
        assert_eq!(src.noise_biome([0, 0, -4]), "minecraft:desert");
        assert_eq!(src.noise_biome([0, 0, 0]), "minecraft:ocean");
    }
    #[test]
    fn defaults_and_round_trip() {
        let cat = catalog();
        let minimal = r#"{"type":"yoake:grid","cell_size":4,"width":1,"depth":1,
            "cells":["minecraft:plains"],"outside":"minecraft:ocean"}"#;
        let def = cat.decode(minimal).unwrap();
        let BiomeSourceDefinition::Grid { origin, seed, .. } = &def else { panic!() };
        assert_eq!((*origin, *seed), ([0, 0], 0));
        for json in [minimal, GRID] {
            let def = cat.decode(json).unwrap();
            let encoded = cat.encode(&def).unwrap();
            assert_eq!(cat.decode(&encoded).unwrap(), def);
            assert!(encoded.contains("\"yoake:grid\""));
        }
    }
    #[test]
    fn rejects_bad_grids() {
        let cat = catalog();
        let bad = |from: &str, to: &str| {
            let json = GRID.replace(from, to);
            assert!(cat.decode(&json).is_err(), "{to}");
        };
        bad("\"cell_size\":16", "\"cell_size\":6");
        bad("\"cell_size\":16", "\"cell_size\":0");
        bad("[-16,-16]", "[-15,-16]");
        bad("[-16,-16]", "[1]");
        bad("\"width\":2", "\"width\":3");
        bad("\"width\":2", "\"width\":0");
        bad("minecraft:desert", "minecraft:nope");
        bad("\"minecraft:ocean\"", "\"minecraft:nope\"");
        assert!(cat.decode(&GRID.replace(",\"outside\":\"minecraft:ocean\"", "")).is_err());
    }
    #[test]
    fn vanilla_types_unchanged() {
        let cat = catalog();
        let fixed = cat
            .decode(r#"{"type":"minecraft:fixed","biome":"minecraft:plains"}"#)
            .unwrap();
        assert_eq!(fixed, BiomeSourceDefinition::Fixed("minecraft:plains".into()));
        assert_eq!(
            cat.encode(&fixed).unwrap(),
            r#"{"type":"minecraft:fixed","biome":"minecraft:plains"}"#
        );
    }
}
