//! The settings of a container, as edited in settings mode.

use serde_json::Value;

use crate::editor::Editor;
use crate::input::TextInput;

/// Whether documents expire, and when.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeToLive {
    /// Documents never expire.
    Off,
    /// Documents expire only when they say when, with their own `ttl`.
    NoDefault,
    /// Documents expire after the seconds typed in, unless they say otherwise.
    Seconds,
}

/// How the container reads spatial data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Geospatial {
    /// Round-earth coordinates, as longitude and latitude.
    Geography,
    /// Flat coordinates.
    Geometry,
}

/// A container's settings, with the edits made to them.
#[derive(Debug)]
pub struct ContainerSettings {
    /// The properties as the service last reported them.
    original: Value,
    pub ttl: TimeToLive,
    /// The seconds after which documents expire, as typed.
    pub seconds: TextInput,
    pub geospatial: Geospatial,
    /// The indexing policy, as JSON.
    pub indexing: Editor,
    /// The computed properties, as a JSON array.
    pub computed: Editor,
}

impl ContainerSettings {
    /// The settings in the properties the service keeps for a container.
    pub fn from_properties(properties: &Value) -> Self {
        let mut seconds = TextInput::default();
        let ttl = match properties.get("defaultTtl").and_then(Value::as_i64) {
            None => TimeToLive::Off,
            Some(-1) => TimeToLive::NoDefault,
            Some(n) => {
                n.to_string().chars().for_each(|c| seconds.insert(c));
                TimeToLive::Seconds
            }
        };
        let geospatial = match properties.pointer("/geospatialConfig/type") {
            Some(Value::String(kind)) if kind == "Geometry" => Geospatial::Geometry,
            _ => Geospatial::Geography,
        };
        let empty = Value::Array(Vec::new());
        let computed = properties.get("computedProperties").unwrap_or(&empty);
        ContainerSettings {
            indexing: json_editor(&properties["indexingPolicy"]),
            computed: json_editor(computed),
            original: properties.clone(),
            ttl,
            seconds,
            geospatial,
        }
    }

    /// The partition key paths, and whether there is more than one level of them.
    pub fn partition_key(&self) -> (String, &'static str) {
        let key = &self.original["partitionKey"];
        let paths: Vec<&str> = key["paths"]
            .as_array()
            .map(|paths| paths.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let note = if paths.len() > 1 || key["kind"] == "MultiHash" {
            "Hierarchically partitioned container."
        } else {
            "Non-hierarchically partitioned container."
        };
        (paths.join(", "), note)
    }
}

/// An editor holding a value as pretty-printed JSON, with the cursor at its start.
fn json_editor(value: &Value) -> Editor {
    let mut editor = Editor::default();
    editor.set_text(&serde_json::to_string_pretty(value).unwrap_or_default());
    editor.move_to(0, 0);
    editor
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_whether_and_when_documents_expire() {
        let off = ContainerSettings::from_properties(&json!({"id": "drivers"}));
        let no_default = ContainerSettings::from_properties(&json!({"defaultTtl": -1}));
        let seconds = ContainerSettings::from_properties(&json!({"defaultTtl": 3600}));

        assert_eq!(off.ttl, TimeToLive::Off);
        assert_eq!(no_default.ttl, TimeToLive::NoDefault);
        assert_eq!(seconds.ttl, TimeToLive::Seconds);
        assert_eq!(seconds.seconds.text(), "3600");
        assert_eq!(off.seconds.text(), "");
    }

    #[test]
    fn reads_the_geospatial_type_which_is_geography_unless_set() {
        let unset = ContainerSettings::from_properties(&json!({}));
        let geometry =
            ContainerSettings::from_properties(&json!({"geospatialConfig": {"type": "Geometry"}}));

        assert_eq!(unset.geospatial, Geospatial::Geography);
        assert_eq!(geometry.geospatial, Geospatial::Geometry);
    }

    #[test]
    fn describes_the_partition_key_and_whether_it_is_hierarchical() {
        let single = ContainerSettings::from_properties(&json!({
            "partitionKey": {"paths": ["/driverId"], "kind": "Hash"}
        }));
        let hierarchical = ContainerSettings::from_properties(&json!({
            "partitionKey": {"paths": ["/tenantId", "/userId"], "kind": "MultiHash"}
        }));

        assert_eq!(
            single.partition_key(),
            ("/driverId".to_string(), "Non-hierarchically partitioned container.")
        );
        assert_eq!(
            hierarchical.partition_key(),
            ("/tenantId, /userId".to_string(), "Hierarchically partitioned container.")
        );
    }

    #[test]
    fn puts_the_indexing_policy_and_computed_properties_in_editors_as_json() {
        let settings = ContainerSettings::from_properties(&json!({
            "indexingPolicy": {"automatic": true, "includedPaths": [{"path": "/*"}]},
            "computedProperties": [{"name": "cp_lower", "query": "SELECT VALUE LOWER(c.name) FROM c"}]
        }));

        assert_eq!(
            settings.indexing.text(),
            "{\n  \"automatic\": true,\n  \"includedPaths\": [\n    {\n      \"path\": \"/*\"\n    }\n  ]\n}"
        );
        assert_eq!(
            settings.computed.text(),
            "[\n  {\n    \"name\": \"cp_lower\",\n    \"query\": \"SELECT VALUE LOWER(c.name) FROM c\"\n  }\n]"
        );
    }

    #[test]
    fn starts_without_computed_properties_as_an_empty_list() {
        let settings = ContainerSettings::from_properties(&json!({}));

        assert_eq!(settings.computed.text(), "[]");
    }
}
