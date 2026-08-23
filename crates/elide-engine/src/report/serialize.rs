//! The `serde::Serialize` impl for [`Report`]: the part-grouped
//! `{ body, parts }` wire view a review layer consumes. Each group is tagged
//! with its [`modality`](elide_core::modality::Modality::NAME) so the
//! orchestrator can route it back on [`deserialize_report`].
//!
//! [`Report`]: super::Report
//! [`deserialize_report`]: crate::Orchestrator::deserialize_report

use std::collections::HashMap;

use super::{EntityGroup, Report};

impl serde::Serialize for Report {
    /// Serialize to `{ body: {modality, entities}, parts: { id: {modality,
    /// entities} } }`. `body` is null when no body pipeline ran. Each group
    /// carries its modality name so it can be parsed back into the right
    /// `Vec<Entity<M>>`.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        // Adapt an erased group to a Serialize value: `{ modality, entities }`.
        struct Group<'a>(&'a dyn EntityGroup);
        impl serde::Serialize for Group<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                // The entities serialize through erasure; `modality` tags which
                // `M` to parse them back as.
                struct Entities<'a>(&'a dyn EntityGroup);
                impl serde::Serialize for Entities<'_> {
                    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                        erased_serde::serialize(self.0, s)
                    }
                }
                let mut state = s.serialize_struct("Group", 2)?;
                state.serialize_field("modality", self.0.modality_name())?;
                state.serialize_field("entities", &Entities(self.0))?;
                state.end()
            }
        }

        let parts: HashMap<&str, Group<'_>> = self
            .parts
            .iter()
            .map(|(id, p)| (id.as_str(), Group(p.entities.as_ref())))
            .collect();

        // `usage` is the third field only under the `usage` feature.
        #[cfg(feature = "usage")]
        let field_count = 3;
        #[cfg(not(feature = "usage"))]
        let field_count = 2;

        let mut state = serializer.serialize_struct("Report", field_count)?;
        state.serialize_field(
            "body",
            &self.body.as_ref().map(|b| Group(b.entities.as_ref())),
        )?;
        state.serialize_field("parts", &parts)?;
        #[cfg(feature = "usage")]
        state.serialize_field("usage", &self.usage)?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{AuditEvent, AuditLog, PatternEvent};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::Report;

    fn text_entity(label: &str) -> Entity<Text> {
        let loc = TextLocation::new(0, 4);
        let event = AuditEvent::pattern("t", Confidence::MAX, loc.clone(), PatternEvent::default());
        Entity::new(
            LabelRef::new(label),
            loc,
            Confidence::MAX,
            AuditLog::new(event),
        )
    }

    #[test]
    fn serializes_body_to_grouped_view() {
        // The part-grouped `{ body, parts }` shape is exercised end to end
        // (with a real container) in the docx integration test; here we
        // check the body group and the empty-parts shape directly.
        let report = Report::new().insert_body::<Text>(vec![text_entity("EMAIL_ADDRESS")]);

        let value = serde_json::to_value(&report).unwrap();
        // body is a `{ modality, entities }` group; parts is an object.
        assert_eq!(value["body"]["modality"], "text");
        assert_eq!(value["body"]["entities"][0]["label"], "EMAIL_ADDRESS");
        assert!(value["parts"].is_object());
        assert_eq!(value["parts"].as_object().unwrap().len(), 0);

        // No body pipeline ran → body is null.
        let empty = serde_json::to_value(Report::new()).unwrap();
        assert!(empty["body"].is_null());
    }

    #[cfg(feature = "usage")]
    #[test]
    fn serializes_usage_entries() {
        use std::time::Duration;

        use elide_core::recognition::{RecognizerId, Usage};

        let mut report = Report::new();
        report.usage.extend([Usage::new(
            RecognizerId::new("elide-pattern", "1"),
            Duration::from_millis(5),
            3,
        )]);

        let value = serde_json::to_value(&report).unwrap();
        let entries = value["usage"]["entries"].as_array().expect("usage array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"]["name"], "elide-pattern");
        assert_eq!(entries[0]["duration"], 5);
        assert_eq!(entries[0]["count"], 3);
        // An empty report still carries an (empty) usage array.
        let empty = serde_json::to_value(Report::new()).unwrap();
        assert!(empty["usage"]["entries"].as_array().unwrap().is_empty());
    }
}
