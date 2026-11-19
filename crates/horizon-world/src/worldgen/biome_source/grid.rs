//! `yoake:grid`: a rectangle of biome cells given by a mod or data pack.
//! Climate-free, so a sample is two divisions and an array read.
use std::collections::BTreeSet;

/// Most cells one grid may hold (256 x 256).
pub const MAX_GRID_CELLS: usize = 65536;
/// Quarts (4 blocks) per biome cell column, as chunk sections store biomes.
const QUART: i32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridError {
    CellSize,
    Origin,
    Extent,
    CellCount,
    TooManyBiomes,
}
impl std::fmt::Display for GridError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::CellSize => "grid cell_size must be a positive multiple of 4",
            Self::Origin => "grid origin must be multiples of 4",
            Self::Extent => "grid width and depth must be 1..=256 cells",
            Self::CellCount => "grid cells must list width * depth biomes",
            Self::TooManyBiomes => "grid uses too many distinct biomes",
        })
    }
}
impl std::error::Error for GridError {}

/// Cell `(i, j)` covers blocks `origin + i * cell_size ..` on X and Z; cells
/// are stored row by row (X fastest). Anything outside the rectangle, and
/// every height, takes `outside`.
#[derive(Clone, Debug)]
pub struct GridBiomeSource {
    /// Cell edge and origin in quarts.
    cell_quarts: i32,
    origin_quarts: [i32; 2],
    width: i32,
    depth: i32,
    palette: Vec<String>,
    cells: Vec<u16>,
    outside: u16,
}
impl GridBiomeSource {
    /// `cell_size` and `origin` are in blocks. `cells.len()` must be
    /// `width * depth`.
    pub fn new(
        cell_size: i32,
        origin: [i32; 2],
        width: i32,
        depth: i32,
        cells: &[String],
        outside: &str,
    ) -> Result<Self, GridError> {
        if cell_size < QUART || cell_size % QUART != 0 {
            return Err(GridError::CellSize);
        }
        if origin.iter().any(|v| v % QUART != 0) {
            return Err(GridError::Origin);
        }
        if !(1..=256).contains(&width) || !(1..=256).contains(&depth) {
            return Err(GridError::Extent);
        }
        if cells.len() != (width as usize) * (depth as usize) {
            return Err(GridError::CellCount);
        }
        let mut palette: Vec<String> = Vec::new();
        let mut index_of = |name: &str| -> Result<u16, GridError> {
            if let Some(i) = palette.iter().position(|p| p == name) {
                return Ok(i as u16);
            }
            if palette.len() >= u16::MAX as usize {
                return Err(GridError::TooManyBiomes);
            }
            palette.push(name.to_owned());
            Ok((palette.len() - 1) as u16)
        };
        let outside = index_of(outside)?;
        let cells = cells
            .iter()
            .map(|name| index_of(name))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            cell_quarts: cell_size / QUART,
            origin_quarts: [origin[0] / QUART, origin[1] / QUART],
            width,
            depth,
            palette,
            cells,
            outside,
        })
    }
    /// The biome at a quart position (the height is ignored). Any `i32` is
    /// valid, negative included.
    pub fn noise_biome(&self, quart: [i32; 3]) -> &str {
        let cx = (i64::from(quart[0]) - i64::from(self.origin_quarts[0]))
            .div_euclid(i64::from(self.cell_quarts));
        let cz = (i64::from(quart[2]) - i64::from(self.origin_quarts[1]))
            .div_euclid(i64::from(self.cell_quarts));
        let index = if (0..i64::from(self.width)).contains(&cx)
            && (0..i64::from(self.depth)).contains(&cz)
        {
            self.cells[(cz * i64::from(self.width) + cx) as usize]
        } else {
            self.outside
        };
        &self.palette[usize::from(index)]
    }
    /// The outside biome first, then the others in first-use order.
    pub fn biomes(&self) -> impl Iterator<Item = &str> {
        self.palette.iter().map(String::as_str)
    }
    pub fn possible_biomes(&self) -> BTreeSet<&str> {
        self.biomes().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }
    /// 2 x 2 cells of 16 blocks at origin (-16, -16): plains/desert over
    /// forest/taiga, `ocean` outside.
    fn grid() -> GridBiomeSource {
        GridBiomeSource::new(
            16,
            [-16, -16],
            2,
            2,
            &s(&["plains", "desert", "forest", "taiga"]),
            "ocean",
        )
        .unwrap()
    }
    fn at(g: &GridBiomeSource, bx: i32, bz: i32) -> &str {
        g.noise_biome([bx >> 2, 0, bz >> 2])
    }
    #[test]
    fn cells_and_boundaries() {
        let g = grid();
        assert_eq!(at(&g, -16, -16), "plains");
        assert_eq!(at(&g, -1, -1), "plains");
        assert_eq!(at(&g, 0, -1), "desert");
        assert_eq!(at(&g, -1, 0), "forest");
        assert_eq!(at(&g, 0, 0), "taiga");
        assert_eq!(at(&g, 15, 15), "taiga");
    }
    #[test]
    fn outside_and_negative() {
        let g = grid();
        assert_eq!(at(&g, 16, 0), "ocean");
        assert_eq!(at(&g, 0, 16), "ocean");
        assert_eq!(at(&g, -17, -16), "ocean");
        assert_eq!(at(&g, -16, -17), "ocean");
        assert_eq!(at(&g, i32::MIN, i32::MAX & !3), "ocean");
        // Height does not matter.
        assert_eq!(g.noise_biome([-4, 1000, -4]), "plains");
        assert_eq!(g.noise_biome([-4, -1000, -4]), "plains");
    }
    #[test]
    fn row_major_x_fastest() {
        let g = GridBiomeSource::new(4, [0, 0], 3, 1, &s(&["a", "b", "c"]), "z").unwrap();
        assert_eq!([0, 1, 2].map(|x| g.noise_biome([x, 0, 0])), ["a", "b", "c"]);
        assert_eq!(g.noise_biome([3, 0, 0]), "z");
        assert_eq!(g.noise_biome([0, 0, 1]), "z");
    }
    #[test]
    fn validation() {
        let one = s(&["a"]);
        let make = |c, o, w, d: i32, cells: &[String]| GridBiomeSource::new(c, o, w, d, cells, "z");
        assert_eq!(make(6, [0, 0], 1, 1, &one).unwrap_err(), GridError::CellSize);
        assert_eq!(make(0, [0, 0], 1, 1, &one).unwrap_err(), GridError::CellSize);
        assert_eq!(make(-16, [0, 0], 1, 1, &one).unwrap_err(), GridError::CellSize);
        assert_eq!(make(16, [2, 0], 1, 1, &one).unwrap_err(), GridError::Origin);
        assert_eq!(make(16, [0, 0], 0, 1, &one).unwrap_err(), GridError::Extent);
        assert_eq!(make(16, [0, 0], 257, 1, &one).unwrap_err(), GridError::Extent);
        assert_eq!(make(16, [0, 0], 2, 1, &one).unwrap_err(), GridError::CellCount);
    }
    #[test]
    fn possible_biomes_include_outside() {
        let g = grid();
        assert_eq!(
            g.possible_biomes().into_iter().collect::<Vec<_>>(),
            ["desert", "forest", "ocean", "plains", "taiga"]
        );
    }
}
