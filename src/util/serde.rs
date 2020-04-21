use std::str::FromStr;

use serde::Deserialize;

pub fn deser_parse<'de, D: serde::Deserializer<'de>, T: FromStr<Err = E>, E: std::fmt::Display>(
    deserializer: D,
) -> Result<T, D::Error> {
    let s: &'de str = Deserialize::deserialize(deserializer)?;
    s.parse().map_err(serde::de::Error::custom)
}
