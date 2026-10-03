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

/// A setting on the settings tab that keys can change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    Ttl,
    /// The seconds after which documents expire, only while they are asked for.
    Seconds,
    Geospatial,
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
    /// The setting keys change on the settings tab.
    pub field: Field,
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
            field: Field::Ttl,
        }
    }

    /// Moves to the next setting on the settings tab, if there is one.
    pub fn next_field(&mut self) {
        self.field = match self.field {
            Field::Ttl if self.ttl == TimeToLive::Seconds => Field::Seconds,
            Field::Ttl | Field::Seconds | Field::Geospatial => Field::Geospatial,
        };
    }

    /// Moves to the setting before on the settings tab, if there is one.
    pub fn previous_field(&mut self) {
        self.field = match self.field {
            Field::Geospatial if self.ttl == TimeToLive::Seconds => Field::Seconds,
            Field::Ttl | Field::Seconds | Field::Geospatial => Field::Ttl,
        };
    }

    /// Picks the next option of the setting, back to the first after the last.
    pub fn next_choice(&mut self) {
        match self.field {
            Field::Ttl => {
                self.ttl = match self.ttl {
                    TimeToLive::Off => TimeToLive::NoDefault,
                    TimeToLive::NoDefault => TimeToLive::Seconds,
                    TimeToLive::Seconds => TimeToLive::Off,
                }
            }
            Field::Geospatial => self.toggle_geospatial(),
            Field::Seconds => {}
        }
    }

    /// Picks the option before of the setting, round to the last from the first.
    pub fn previous_choice(&mut self) {
        match self.field {
            Field::Ttl => {
                self.ttl = match self.ttl {
                    TimeToLive::Off => TimeToLive::Seconds,
                    TimeToLive::NoDefault => TimeToLive::Off,
                    TimeToLive::Seconds => TimeToLive::NoDefault,
                }
            }
            Field::Geospatial => self.toggle_geospatial(),
            Field::Seconds => {}
        }
    }

    fn toggle_geospatial(&mut self) {
        self.geospatial = match self.geospatial {
            Geospatial::Geography => Geospatial::Geometry,
            Geospatial::Geometry => Geospatial::Geography,
        };
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
        for (editor, tab) in self.json_tabs() {
            if let Some(value) = tab.parse(editor, &tab.was(&self.original))? {
                object.insert(tab.name.into(), value);
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

    /// Whether the settings a tab shows were edited since they were loaded or saved.
    pub fn modified(&self, tab: SettingsTab) -> bool {
        if tab == SettingsTab::Settings {
            let was = ContainerSettings::from_properties(&self.original);
            let seconds_changed =
                self.ttl == TimeToLive::Seconds && self.seconds.text() != was.seconds.text();
            return self.ttl != was.ttl || seconds_changed || self.geospatial != was.geospatial;
        }
        self.json_tabs()
            .into_iter()
            .filter(|(_, json)| json.tab == tab)
            .any(|(editor, json)| json.parse(editor, &json.was(&self.original)) != Ok(None))
    }

    /// The editors of the tabs written as JSON, with what they hold.
    fn json_tabs(&self) -> [(&Editor, JsonTab); 2] {
        [
            (&self.indexing, INDEXING_POLICY),
            (&self.computed, COMPUTED_PROPERTIES),
        ]
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

/// A tab whose setting is written as JSON of one shape.
struct JsonTab {
    tab: SettingsTab,
    /// The property holding the setting.
    name: &'static str,
    /// What the setting is when the property is missing.
    unset: fn() -> Value,
    /// What the setting is called, and the verb that goes with it.
    subject: (&'static str, &'static str),
    /// The shape the JSON must have, and how to tell.
    shape: (&'static str, fn(&Value) -> bool),
}

const INDEXING_POLICY: JsonTab = JsonTab {
    tab: SettingsTab::IndexingPolicy,
    name: "indexingPolicy",
    unset: || Value::Null,
    subject: ("The indexing policy", "is"),
    shape: ("a JSON object", Value::is_object),
};

const COMPUTED_PROPERTIES: JsonTab = JsonTab {
    tab: SettingsTab::ComputedProperties,
    name: "computedProperties",
    unset: || Value::Array(Vec::new()),
    subject: ("The computed properties", "are"),
    shape: ("a JSON array", Value::is_array),
};

impl JsonTab {
    /// The setting in the properties.
    fn was(&self, properties: &Value) -> Value {
        properties
            .get(self.name)
            .cloned()
            .unwrap_or_else(self.unset)
    }

    /// The JSON in the editor, or `None` when it holds the value as it was.
    fn parse(&self, editor: &Editor, was: &Value) -> Result<Option<Value>, SettingsError> {
        let (subject, verb) = self.subject;
        let (shape, has_shape) = self.shape;
        let value: Value = serde_json::from_str(&editor.text())
            .map_err(|error| self.error(format!("{subject} {verb} not valid JSON: {error}")))?;
        if value == *was {
            return Ok(None);
        }
        if !has_shape(&value) {
            return Err(self.error(format!("{subject} must be {shape}.")));
        }
        Ok(Some(value))
    }

    fn error(&self, message: String) -> SettingsError {
        SettingsError {
            tab: self.tab,
            message,
        }
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

    fn retype(editor: &mut Editor, text: &str) {
        editor.set_text(text);
    }

    #[test]
    fn writes_back_the_edited_indexing_policy() {
        let mut settings = ContainerSettings::from_properties(&json!({
            "indexingPolicy": {"automatic": true}
        }));

        retype(&mut settings.indexing, r#"{"automatic": false}"#);

        assert_eq!(
            settings.to_properties().unwrap()["indexingPolicy"],
            json!({"automatic": false})
        );
    }

    #[test]
    fn says_where_the_indexing_policy_is_not_valid_json() {
        let mut settings = ContainerSettings::from_properties(&json!({"indexingPolicy": {}}));

        retype(&mut settings.indexing, "{\n  \"automatic\": tru}");

        assert_eq!(
            settings.to_properties(),
            Err(SettingsError {
                tab: SettingsTab::IndexingPolicy,
                message:
                    "The indexing policy is not valid JSON: expected ident at line 2 column 19"
                        .into()
            })
        );
    }

    #[test]
    fn requires_the_indexing_policy_to_be_an_object() {
        let mut settings = ContainerSettings::from_properties(&json!({"indexingPolicy": {}}));

        retype(&mut settings.indexing, "[]");

        assert_eq!(
            settings.to_properties(),
            Err(SettingsError {
                tab: SettingsTab::IndexingPolicy,
                message: "The indexing policy must be a JSON object.".into()
            })
        );
    }

    #[test]
    fn writes_back_the_edited_computed_properties() {
        let mut settings = ContainerSettings::from_properties(&json!({"id": "drivers"}));
        assert_eq!(settings.to_properties().unwrap(), json!({"id": "drivers"}));

        retype(
            &mut settings.computed,
            r#"[{"name": "cp", "query": "SELECT VALUE 1 FROM c"}]"#,
        );

        assert_eq!(
            settings.to_properties().unwrap()["computedProperties"],
            json!([{"name": "cp", "query": "SELECT VALUE 1 FROM c"}])
        );
    }

    #[test]
    fn requires_the_computed_properties_to_be_a_json_array() {
        let mut settings = ContainerSettings::from_properties(&json!({}));

        retype(&mut settings.computed, "[");
        assert_eq!(
            settings.to_properties(),
            Err(SettingsError {
                tab: SettingsTab::ComputedProperties,
                message: "The computed properties are not valid JSON: \
                          EOF while parsing a list at line 1 column 1"
                    .into()
            })
        );
        retype(&mut settings.computed, "{}");
        assert_eq!(
            settings.to_properties(),
            Err(SettingsError {
                tab: SettingsTab::ComputedProperties,
                message: "The computed properties must be a JSON array.".into()
            })
        );
    }

    #[test]
    fn tells_which_tabs_have_changes() {
        let properties = json!({"defaultTtl": 60, "indexingPolicy": {"automatic": true}});
        let tabs = [
            SettingsTab::Settings,
            SettingsTab::IndexingPolicy,
            SettingsTab::ComputedProperties,
        ];
        let changed = |settings: &ContainerSettings| -> Vec<SettingsTab> {
            tabs.into_iter()
                .filter(|tab| settings.modified(*tab))
                .collect()
        };
        let mut settings = ContainerSettings::from_properties(&properties);
        assert_eq!(changed(&settings), []);

        type_seconds(&mut settings, "61");
        assert_eq!(changed(&settings), [SettingsTab::Settings]);
        type_seconds(&mut settings, "60");
        settings.geospatial = Geospatial::Geometry;
        assert_eq!(changed(&settings), [SettingsTab::Settings]);
        settings.geospatial = Geospatial::Geography;

        retype(&mut settings.indexing, r#"{ "automatic": true }"#);
        assert_eq!(changed(&settings), [], "only reformatted");
        retype(&mut settings.indexing, r#"{"automatic": tr"#);
        retype(&mut settings.computed, "[{}]");
        assert_eq!(
            changed(&settings),
            [SettingsTab::IndexingPolicy, SettingsTab::ComputedProperties]
        );
    }

    #[test]
    fn moves_through_the_fields_reaching_the_seconds_only_while_they_count() {
        let mut settings = ContainerSettings::from_properties(&json!({}));
        assert_eq!(settings.field, Field::Ttl);

        settings.next_field();
        assert_eq!(settings.field, Field::Geospatial);
        settings.next_field();
        assert_eq!(settings.field, Field::Geospatial);
        settings.previous_field();
        assert_eq!(settings.field, Field::Ttl);

        settings.ttl = TimeToLive::Seconds;
        settings.next_field();
        assert_eq!(settings.field, Field::Seconds);
        settings.next_field();
        settings.previous_field();
        assert_eq!(settings.field, Field::Seconds);
        settings.previous_field();
        settings.previous_field();
        assert_eq!(settings.field, Field::Ttl);
    }

    #[test]
    fn changes_the_choice_of_the_field_round_its_options() {
        let mut settings = ContainerSettings::from_properties(&json!({}));

        settings.next_choice();
        assert_eq!(settings.ttl, TimeToLive::NoDefault);
        settings.next_choice();
        assert_eq!(settings.ttl, TimeToLive::Seconds);
        settings.next_choice();
        assert_eq!(settings.ttl, TimeToLive::Off);
        settings.previous_choice();
        assert_eq!(settings.ttl, TimeToLive::Seconds);

        settings.field = Field::Geospatial;
        settings.next_choice();
        assert_eq!(settings.geospatial, Geospatial::Geometry);
        settings.previous_choice();
        assert_eq!(settings.geospatial, Geospatial::Geography);
    }
}
