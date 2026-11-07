use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// The crate's folder inside the data folder.
    #[serde(rename = "crate")]
    pub crate_dir: String,
    /// The cargo profile to build with.
    pub profile: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            crate_dir: "copperscript".to_owned(),
            profile: "release".to_owned(),
        }
    }
}

impl Settings {
    /// Parses the settings argument. Regolith passes none when the profile sets no settings.
    pub fn parse(argument: Option<&str>) -> Result<Self> {
        match argument {
            None => Ok(Self::default()),
            Some(json) => serde_json::from_str(json)
                .with_context(|| format!("cannot read the filter settings `{json}`")),
        }
    }

    /// The cargo arguments that select the profile.
    pub fn profile_args(&self) -> Vec<String> {
        if self.profile == "release" {
            vec!["--release".to_owned()]
        } else {
            vec!["--profile".to_owned(), self.profile.clone()]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_without_settings() {
        let settings = Settings::parse(None).unwrap();
        assert_eq!(settings.crate_dir, "copperscript");
        assert_eq!(settings.profile, "release");
    }

    #[test]
    fn defaults_for_missing_keys() {
        assert_eq!(Settings::parse(Some("{}")).unwrap(), Settings::default());
        let settings = Settings::parse(Some(r#"{"crate": "plugin"}"#)).unwrap();
        assert_eq!(settings.crate_dir, "plugin");
        assert_eq!(settings.profile, "release");
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(Settings::parse(Some(r#"{"profle": "dev"}"#)).is_err());
    }

    #[test]
    fn profile_args() {
        assert_eq!(Settings::default().profile_args(), ["--release"]);
        let dev = Settings::parse(Some(r#"{"profile": "dev"}"#)).unwrap();
        assert_eq!(dev.profile_args(), ["--profile", "dev"]);
    }
}
