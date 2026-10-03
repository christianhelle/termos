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

/// The tabs of settings mode, each showing some of the settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsTab {
    /// Time to live, the geospatial type and the partition key.
    Settings,
    IndexingPolicy,
    ComputedProperties,
}

/// Why edited settings cannot be saved, and on which tab.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsError {
    pub tab: SettingsTab,
    pub message: String,
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

    /// The properties to replace the container's with, holding the edited settings.
    pub fn to_properties(&self) -> Result<Value, SettingsError> {
        let mut properties = self.original.clone();
        let object = properties
            .as_object_mut()
            .expect("container properties are an object");
        match self.ttl {
            TimeToLive::Off => {
                object.remove("defaultTtl");
            }
            TimeToLive::NoDefault => {
                object.insert("defaultTtl".into(), Value::from(-1));
            }
            TimeToLive::Seconds => {
                let seconds = self
                    .seconds
                    .text()
                    .parse::<i32>()
                    .ok()
                    .filter(|seconds| *seconds > 0)
                    .ok_or_else(|| SettingsError {
                        tab: SettingsTab::Settings,
                        message:
                            "Time to live must be a whole number of seconds from 1 to 2147483647."
                                .into(),
                    })?;
                object.insert("defaultTtl".into(), Value::from(seconds));
            }
        }
        let kind = match self.geospatial {
            Geospatial::Geography => "Geography",
            Geospatial::Geometry => "Geometry",
        };
        // A container that never set it reads geography, so it is only written when set or changed
        if object.contains_key("geospatialConfig") || self.geospatial != Geospatial::Geography {
            object.insert(
                "geospatialConfig".into(),
                serde_json::json!({ "type": kind }),
            );
        }
        Ok(properties)
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
            (
                "/driverId".to_string(),
                "Non-hierarchically partitioned container."
            )
        );
        assert_eq!(
            hierarchical.partition_key(),
            (
                "/tenantId, /userId".to_string(),
                "Hierarchically partitioned container."
            )
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

    fn type_seconds(settings: &mut ContainerSettings, text: &str) {
        settings.seconds = TextInput::default();
        text.chars().for_each(|c| settings.seconds.insert(c));
    }

    #[test]
    fn writes_back_whether_and_when_documents_expire() {
        let mut settings =
            ContainerSettings::from_properties(&json!({"id": "drivers", "defaultTtl": 3600}));

        settings.ttl = TimeToLive::Off;
        assert_eq!(settings.to_properties().unwrap(), json!({"id": "drivers"}));
        settings.ttl = TimeToLive::NoDefault;
        assert_eq!(settings.to_properties().unwrap()["defaultTtl"], -1);
        settings.ttl = TimeToLive::Seconds;
        type_seconds(&mut settings, "60");
        assert_eq!(settings.to_properties().unwrap()["defaultTtl"], 60);
    }

    #[test]
    fn rejects_seconds_that_are_not_a_positive_whole_number() {
        let mut settings = ContainerSettings::from_properties(&json!({"defaultTtl": 3600}));

        for typed in ["", "0", "1.5", "2147483648"] {
            type_seconds(&mut settings, typed);

            assert_eq!(
                settings.to_properties(),
                Err(SettingsError {
                    tab: SettingsTab::Settings,
                    message: "Time to live must be a whole number of seconds from 1 to 2147483647."
                        .into()
                }),
                "{typed:?}"
            );
        }
    }

    #[test]
    fn writes_back_the_geospatial_type() {
        let mut settings = ContainerSettings::from_properties(&json!({"id": "drivers"}));

        settings.geospatial = Geospatial::Geometry;

        assert_eq!(
            settings.to_properties().unwrap()["geospatialConfig"],
            json!({"type": "Geometry"})
        );
    }
}
