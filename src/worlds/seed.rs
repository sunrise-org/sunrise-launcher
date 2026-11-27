//! A world's seed, read from its level.dat: `Data.WorldGenSettings.seed` since 1.16, and
//! `Data.RandomSeed` before it.
use super::*;

/// The seed of the world in `world` (its folder), if its level.dat has one.
pub fn seed(world: &Path) -> Option<i64> {
    let root = level_dat(&world.join("level.dat"))?;
    let data = root.get("Data")?;
    let long = |tag: Option<&Tag>| match tag {
        Some(Tag::Long(seed)) => Some(*seed),
        _ => None,
    };
    long(data.get("WorldGenSettings").and_then(|w| w.get("seed")))
        .or_else(|| long(data.get("RandomSeed")))
}

#[cfg(test)]
mod tests;
