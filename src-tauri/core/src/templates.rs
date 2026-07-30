//! Note templates: the shape a summary is written to.
//!
//! A template is not a document the user fills in — it is a set of section
//! headings handed to the summarizer, so "1:1" produces different notes from
//! "Lecture" off the same transcript. The user picks one per recording (or
//! never picks, and gets [`DEFAULT_ID`]), and re-picking plus a reprocess
//! rewrites the summary to the new shape.
//!
//! The table is `const` on purpose. These ids are written into `meta.json`, so
//! they are a storage contract: renaming one orphans every recording that used
//! it. Adding a template is free; renaming or removing an id is not, and
//! `find` deliberately keeps working for an unknown id by falling back to the
//! default rather than erroring — a recording filed under a template a future
//! version dropped must still open.

/// The template used when a recording has none, and the fallback for an id we
/// no longer recognize.
pub const DEFAULT_ID: &str = "default";

/// One note shape.
pub struct Template {
    /// Stored in `meta.json`. A storage contract — see the module note.
    pub id: &'static str,
    /// What the picker shows.
    pub name: &'static str,
    /// One line under the name, so the user can tell "Standup" from "1:1"
    /// without trying both.
    pub blurb: &'static str,
    /// The section list handed to the summarizer, verbatim.
    pub sections: &'static str,
}

/// Every template, in the order the picker shows them. `default` is first
/// because it is what most recordings want.
pub const TEMPLATES: &[Template] = &[
    Template {
        id: DEFAULT_ID,
        name: "General notes",
        blurb: "A good default for any conversation.",
        sections: "## TL;DR (2-3 sentences)\n## Key points\n## Decisions\n## Action items (checkbox list, each starting with the owner's name and a colon)\n## Open questions",
    },
    Template {
        id: "one_on_one",
        name: "1:1",
        blurb: "A recurring check-in with one person.",
        sections: "## TL;DR (2-3 sentences)\n## What they raised\n## Feedback given and received\n## Blockers\n## Action items (checkbox list, each starting with the owner's name and a colon)\n## Follow up next time",
    },
    Template {
        id: "standup",
        name: "Standup",
        blurb: "Short status round with a team.",
        sections: "## TL;DR (1-2 sentences)\n## Updates by person\n## Blockers\n## Action items (checkbox list, each starting with the owner's name and a colon)",
    },
    Template {
        id: "lecture",
        name: "Lecture or class",
        blurb: "One person teaching. Optimized for studying later.",
        sections: "## TL;DR (2-3 sentences)\n## Main concepts (each with a one-line explanation)\n## Definitions and formulas (verbatim where stated)\n## Examples worked through\n## Likely exam material\n## Action items (checkbox list — readings, problem sets, deadlines)",
    },
    Template {
        id: "client_call",
        name: "Client call",
        blurb: "An external conversation you may need to answer for.",
        sections: "## TL;DR (2-3 sentences)\n## What the client asked for\n## Commitments we made (quote the wording used)\n## Commitments they made\n## Pricing, dates and numbers mentioned\n## Risks and objections\n## Action items (checkbox list, each starting with the owner's name and a colon)",
    },
    Template {
        id: "interview",
        name: "Interview",
        blurb: "Assessing a candidate, or being assessed.",
        sections: "## TL;DR (2-3 sentences)\n## Background as described\n## Answers to each question asked\n## Strengths with evidence from the transcript\n## Concerns with evidence from the transcript\n## Action items (checkbox list, each starting with the owner's name and a colon)",
    },
];

/// The template for `id`, falling back to the default.
///
/// Never `None` for a caller that just wants to summarize: an unknown id means
/// a `meta.json` written by a version that had a template this one dropped,
/// and refusing to summarize that recording would be a worse outcome than
/// summarizing it in the default shape.
pub fn find(id: Option<&str>) -> &'static Template {
    let wanted = id.unwrap_or(DEFAULT_ID);
    TEMPLATES
        .iter()
        .find(|t| t.id == wanted)
        .unwrap_or(&TEMPLATES[0])
}

/// True if `id` names a template this build knows.
///
/// Used to reject a bad id at the *command* boundary, where the user is
/// actively choosing and a silent fallback would look like the picker is
/// broken. Contrast [`find`], which is called at summarize time and must never
/// refuse to produce notes.
pub fn is_known(id: &str) -> bool {
    TEMPLATES.iter().any(|t| t.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_id_resolves_and_is_first() {
        assert_eq!(TEMPLATES[0].id, DEFAULT_ID);
        assert_eq!(find(None).id, DEFAULT_ID);
        assert_eq!(find(Some(DEFAULT_ID)).id, DEFAULT_ID);
    }

    #[test]
    fn every_template_has_a_name_a_blurb_and_sections() {
        for t in TEMPLATES {
            assert!(!t.name.is_empty(), "{} has no name", t.id);
            assert!(!t.blurb.is_empty(), "{} has no blurb", t.id);
            assert!(
                t.sections.starts_with("## "),
                "{} sections should start with a heading: {}",
                t.id,
                t.sections
            );
        }
    }

    /// Ids are written into `meta.json`, so a duplicate would make two
    /// templates indistinguishable on disk.
    #[test]
    fn ids_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for t in TEMPLATES {
            assert!(seen.insert(t.id), "{} is claimed twice", t.id);
        }
    }

    /// Ids go in `meta.json` and travel over JSON to the UI, so keep them to
    /// the boring subset that needs no escaping anywhere.
    #[test]
    fn ids_are_lowercase_snake_case() {
        for t in TEMPLATES {
            assert!(
                t.id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{} is not lowercase snake_case",
                t.id
            );
        }
    }

    /// The one behaviour that protects existing recordings: a template id this
    /// build has never heard of still summarizes, in the default shape.
    #[test]
    fn an_unknown_id_falls_back_instead_of_failing() {
        assert_eq!(find(Some("template_from_the_future")).id, DEFAULT_ID);
        assert_eq!(find(Some("")).id, DEFAULT_ID);
        assert!(!is_known("template_from_the_future"));
    }

    /// Every template must ask for action items, because the UI renders a
    /// checklist for every recording and a template that produced none would
    /// look like the feature was broken rather than inapplicable.
    #[test]
    fn every_template_asks_for_action_items() {
        for t in TEMPLATES {
            assert!(
                t.sections.to_lowercase().contains("action items"),
                "{} produces no action items, so its checklist would be empty",
                t.id
            );
        }
    }

    /// Every template must open with a TL;DR: it is what the library list and
    /// the collapsed card show, so a template without one has nothing to
    /// preview.
    #[test]
    fn every_template_opens_with_a_tldr() {
        for t in TEMPLATES {
            assert!(
                t.sections.starts_with("## TL;DR"),
                "{} does not start with a TL;DR",
                t.id
            );
        }
    }
}
