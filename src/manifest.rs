// Manifest: declarative description of a moof image.
//
// moof.toml defines everything: which type plugins to load,
// which capability vats to spawn, which source files to eval,
// and which capabilities each context gets. No manifest = the
// binary is a blank VM with nothing in it.

use std::collections::HashMap;
use std::path::Path;
use serde::Deserialize;

/// A parsed moof manifest.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub image: ImageConfig,
    #[serde(default)]
    pub types: HashMap<String, String>,
    #[serde(default)]
    pub capabilities: HashMap<String, String>,
    #[serde(default)]
    pub sources: SourcesConfig,
    #[serde(default)]
    pub grants: HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ImageConfig {
    #[serde(default = "default_image_name")]
    pub name: String,
    #[serde(default = "default_image_path")]
    pub path: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SourcesConfig {
    #[serde(default)]
    pub files: Vec<String>,
}

fn default_image_name() -> String { "moof".into() }
fn default_image_path() -> String { ".moof/store".into() }

impl Manifest {
    /// Load from a TOML file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read manifest: {e}"))?;
        toml::from_str(&content)
            .map_err(|e| format!("invalid manifest: {e}"))
    }

    /// The default manifest: full stdlib, built-in capabilities.
    pub fn default() -> Self {
        let mut types = HashMap::new();
        types.insert("core".into(), "builtin:core".into());
        types.insert("numeric".into(), "builtin:numeric".into());
        types.insert("collections".into(), "builtin:collections".into());
        types.insert("effects".into(), "builtin:effects".into());
        types.insert("block".into(), "builtin:block".into());

        let mut capabilities = HashMap::new();
        capabilities.insert("console".into(), "builtin:console".into());
        capabilities.insert("clock".into(), "builtin:clock".into());

        let sources = SourcesConfig {
            files: vec![
                "lib/bootstrap.moof".into(),
                "lib/protocols.moof".into(),
                "lib/comparable.moof".into(),
                "lib/numeric.moof".into(),
                "lib/iterable.moof".into(),
                "lib/indexable.moof".into(),
                "lib/callable.moof".into(),
                "lib/types.moof".into(),
                "lib/error.moof".into(),
                "lib/showable.moof".into(),
                "lib/range.moof".into(),
                "lib/act.moof".into(),
            ],
        };

        let mut grants = HashMap::new();
        grants.insert("repl".into(), vec!["console".into(), "clock".into()]);

        Manifest {
            image: ImageConfig::default(),
            types,
            capabilities,
            sources,
            grants,
        }
    }

    /// Resolve a plugin specifier: "builtin:name" or "path/to/lib.dylib"
    pub fn is_builtin(spec: &str) -> Option<&str> {
        spec.strip_prefix("builtin:")
    }
}
