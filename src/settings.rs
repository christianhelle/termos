//! The settings of a container, as edited in settings mode.

use serde_json::Value;

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

/// A container's settings, with the edits made to them.
#[derive(Debug)]
pub struct ContainerSettings {
    pub ttl: TimeToLive,
    /// The seconds after which documents expire, as typed.
    pub seconds: TextInput,
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
        ContainerSettings { ttl, seconds }
    }
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
}
