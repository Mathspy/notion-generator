use std::{fmt, str::FromStr};

#[derive(Clone, Copy)]
pub enum HeadingAnchors {
    None,
    Icon,
}

#[derive(Debug)]
pub struct HeadingAnchorsParseError;
impl fmt::Display for HeadingAnchorsParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Expected either `none` or `icon`")?;

        Ok(())
    }
}
impl std::error::Error for HeadingAnchorsParseError {}

impl FromStr for HeadingAnchors {
    type Err = HeadingAnchorsParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(HeadingAnchors::None),
            "icon" => Ok(HeadingAnchors::Icon),
            _ => Err(HeadingAnchorsParseError),
        }
    }
}
