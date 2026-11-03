#![allow(dead_code)]
use crate::asset;
use serde_json::{Map, Value, json};
use std::{fs::File, io::Read, path::Path};

#[derive(Clone, Debug, Default)]
pub struct Policy {
    values: Map<String, Value>,
    pub path: Option<String>,
}
const SCHEMA: &[(&str, &[&str])] = &[
    (
        "environment",
        &[
            "max_dimension",
            "max_texels",
            "max_payload_bytes",
            "max_map_bytes",
            "max_owned_bytes",
        ],
    ),
    (
        "cook",
        &[
            "source_bytes",
            "json_bytes",
            "nodes",
            "hierarchy_depth",
            "images",
            "image_decoded_bytes",
            "accessors",
            "buffer_views",
            "scenes",
            "samplers",
            "json_values",
            "manifest_bytes",
        ],
    ),
    (
        "prepared",
        &[
            "max_file_bytes",
            "max_decoded_bytes",
            "max_meshes",
            "max_primitives",
            "max_placements",
            "max_materials",
            "max_textures",
            "max_vertices",
            "max_indices",
            "max_dimension",
        ],
    ),
    (
        "virtual",
        &[
            "max_groups",
            "max_clusters",
            "max_pages",
            "max_metadata_bytes",
            "max_geometry_bytes",
        ],
    ),
    (
        "runtime",
        &[
            "geometry_bytes",
            "metadata_bytes",
            "texture_bytes",
            "bootstrap_bytes",
            "read_slots",
            "uploads_per_tick",
            "max_candidates",
            "max_clusters",
            "error_pixels",
            "io_delay_ms",
        ],
    ),
    (
        "renderer",
        &["max_instances", "max_mesh_bytes", "max_texture_bytes"],
    ),
    ("viewer", &["max_placement_slots"]),
];
impl Policy {
    pub fn args(args: Vec<String>) -> Result<(Self, Vec<String>), String> {
        let mut policy = Self::default();
        let mut rest = Vec::new();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "--limits" {
                if policy.path.is_some() {
                    return Err("duplicate --limits".into());
                }
                let path = args.next().ok_or("--limits requires FILE")?;
                let mut bytes = Vec::new();
                File::open(Path::new(&path))
                    .map_err(|e| format!("--limits {path}: {e}"))?
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("--limits {path}: {e}"))?;
                policy = Self::parse(&bytes)?;
                policy.path = Some(path);
            } else {
                rest.push(arg);
            }
        }
        Ok((policy, rest))
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|e| format!("limits JSON: {e}"))?;
        let values = value.as_object().ok_or("limits must be an object")?.clone();
        for (section, value) in &values {
            let (_, fields) = SCHEMA
                .iter()
                .find(|(s, _)| s == section)
                .ok_or_else(|| format!("unknown limits field {section}"))?;
            let uncapped = matches!(
                section.as_str(),
                "cook" | "prepared" | "virtual" | "viewer" | "environment"
            );
            if uncapped && value == "unlimited" {
                continue;
            }
            let object = value.as_object().ok_or_else(|| {
                format!(
                    "limits.{section} must be an object{}",
                    if uncapped { " or \"unlimited\"" } else { "" }
                )
            })?;
            for (field, value) in object {
                let name = format!("limits.{section}.{field}");
                if !fields.contains(&field.as_str()) {
                    return Err(format!("unknown {name}"));
                }
                if uncapped && value == "unlimited" {
                    continue;
                }
                if field == "error_pixels" {
                    if !value.as_f64().is_some_and(|n| n.is_finite() && n >= 0.) {
                        return Err(format!("{name} must be finite and nonnegative"));
                    }
                } else {
                    let n = value.as_u64().ok_or_else(|| {
                        format!(
                            "{name} must be an unsigned integer{}",
                            if uncapped { " or \"unlimited\"" } else { "" }
                        )
                    })?;
                    if usize::try_from(n).is_err()
                        || (!uncapped
                            && !matches!(field.as_str(), "io_delay_ms" | "max_candidates")
                            && (n == 0 || n == usize::MAX as u64))
                    {
                        return Err(format!(
                            "{name} must fit the host and be positive and finite"
                        ));
                    }
                    if matches!(field.as_str(), "max_dimension" | "max_instances")
                        && u32::try_from(n).is_err()
                    {
                        return Err(format!("{name} exceeds u32"));
                    }
                }
            }
        }
        Ok(Self { values, path: None })
    }
    fn value(&self, section: &str, field: &str) -> Option<&Value> {
        let value = self.values.get(section)?;
        if value == "unlimited" {
            Some(value)
        } else {
            value.get(field)
        }
    }
    pub fn size(&self, section: &str, field: &str, default: usize) -> usize {
        match self.value(section, field) {
            Some(Value::String(_)) => usize::MAX,
            Some(value) => value.as_u64().unwrap() as usize,
            None => default,
        }
    }
    pub fn dimension(&self, section: &str, field: &str, default: u32) -> u32 {
        match self.value(section, field) {
            Some(Value::String(_)) => u32::MAX,
            Some(value) => value.as_u64().unwrap() as u32,
            None => default,
        }
    }
    pub fn real(&self, section: &str, field: &str, default: f64) -> f64 {
        self.value(section, field)
            .map_or(default, |v| v.as_f64().unwrap())
    }
    pub fn prepared(&self) -> asset::Limits {
        let mut out = asset::Limits::default();
        out.max_file_bytes = self.size("prepared", "max_file_bytes", out.max_file_bytes);
        out.max_decoded_bytes = self.size("prepared", "max_decoded_bytes", out.max_decoded_bytes);
        out.max_meshes = self.size("prepared", "max_meshes", out.max_meshes);
        out.max_primitives = self.size("prepared", "max_primitives", out.max_primitives);
        out.max_placements = self.size("prepared", "max_placements", out.max_placements);
        out.max_materials = self.size("prepared", "max_materials", out.max_materials);
        out.max_textures = self.size("prepared", "max_textures", out.max_textures);
        out.max_vertices = self.size("prepared", "max_vertices", out.max_vertices);
        out.max_indices = self.size("prepared", "max_indices", out.max_indices);
        out.max_dimension = self.dimension("prepared", "max_dimension", out.max_dimension);
        out
    }
    pub fn virtual_limits(&self) -> asset::virtual_mesh::VirtualLimits {
        let mut out = asset::virtual_mesh::VirtualLimits {
            prepared: self.prepared(),
            ..Default::default()
        };
        out.max_groups = self.size("virtual", "max_groups", out.max_groups);
        out.max_clusters = self.size("virtual", "max_clusters", out.max_clusters);
        out.max_pages = self.size("virtual", "max_pages", out.max_pages);
        out.max_metadata_bytes = self.size("virtual", "max_metadata_bytes", out.max_metadata_bytes);
        out.max_geometry_bytes = self.size("virtual", "max_geometry_bytes", out.max_geometry_bytes);
        out
    }
    pub fn requested(&self) -> Value {
        json!({"file":self.path,"overrides":self.values})
    }
}
pub fn cap(value: usize) -> Value {
    if value == usize::MAX {
        json!("unlimited")
    } else {
        json!(value)
    }
}
pub fn prepared_json(p: asset::Limits) -> Value {
    json!({
        "max_file_bytes":cap(p.max_file_bytes),
        "max_decoded_bytes":cap(p.max_decoded_bytes),
        "max_meshes":cap(p.max_meshes),
        "max_primitives":cap(p.max_primitives),
        "max_placements":cap(p.max_placements),
        "max_materials":cap(p.max_materials),
        "max_textures":cap(p.max_textures),
        "max_vertices":cap(p.max_vertices),
        "max_indices":cap(p.max_indices),
        "max_dimension":if p.max_dimension == u32::MAX { json!("unlimited") } else {json!(p.max_dimension)},
    })
}
pub fn virtual_json(p: asset::virtual_mesh::VirtualLimits) -> Value {
    json!({
        "max_groups":cap(p.max_groups),
        "max_clusters":cap(p.max_clusters),
        "max_pages":cap(p.max_pages),
        "max_metadata_bytes":cap(p.max_metadata_bytes),
        "max_geometry_bytes":cap(p.max_geometry_bytes),
        "prepared":prepared_json(p.prepared)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_fields_and_uncapped_policy() {
        for text in [
            r#"{"wat":{}}"#,
            r#"{"runtime":{"read_slots":0}}"#,
            r#"{"runtime":"unlimited"}"#,
            r#"{"runtime":{"texture_bytes":"unlimited"}}"#,
            r#"{"prepared":{"max_dimension":4294967296}}"#,
            r#"{"cook":{"source_bytes":-1}}"#,
            r#"{"virtual":{"max_pages":1.5}}"#,
        ] {
            assert!(Policy::parse(text.as_bytes()).is_err(), "{text}");
        }
        let p = Policy::parse(br#"{"prepared":"unlimited","virtual":{"max_pages":2}}"#).unwrap();
        assert_eq!(p.prepared(), asset::Limits::unlimited());
        assert_eq!(p.virtual_limits().max_pages, 2);
        assert!(
            Policy::parse(br#"{"runtime":{"read_slots":0}}"#)
                .unwrap_err()
                .contains("runtime.read_slots")
        );
    }
}
